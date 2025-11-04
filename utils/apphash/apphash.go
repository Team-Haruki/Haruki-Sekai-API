package apphash

import (
	"context"
	"errors"
	"fmt"
	harukiLogger "haruki-sekai-api/utils/logger"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"haruki-sekai-api/utils"

	"github.com/bytedance/sonic"
	"github.com/go-resty/resty/v2"
	"github.com/iancoleman/orderedmap"
)

type HarukiSekaiAppHashUpdater struct {
	sources           []utils.HarukiSekaiAppHashSource
	server            string
	serverVersionPath *string
	client            *resty.Client
	logger            *harukiLogger.Logger
}

func NewAppHashUpdater(sources []utils.HarukiSekaiAppHashSource, server utils.HarukiSekaiServerRegion, versionPath *string) *HarukiSekaiAppHashUpdater {
	return &HarukiSekaiAppHashUpdater{
		sources:           sources,
		server:            string(server),
		serverVersionPath: versionPath,
		client: func() *resty.Client {
			cli := resty.New()
			cli.SetTimeout(30 * time.Second)
			return cli
		}(),
		logger: harukiLogger.NewLogger(fmt.Sprintf("HarukiAppHashUpdater%s", strings.ToUpper(string(server))), "INFO", nil),
	}
}

func getFirstStr(om *orderedmap.OrderedMap, keys ...string) string {
	for _, k := range keys {
		if v, ok := om.Get(k); ok {
			if s, ok := v.(string); ok && s != "" {
				return s
			}
		}
	}
	return ""
}

func (a *HarukiSekaiAppHashUpdater) readAppInfoFromFile(source utils.HarukiSekaiAppHashSource, filename string) (*utils.HarukiSekaiAppInfo, error) {
	path := filepath.Join(source.Dir, filename)
	data, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, fs.ErrNotExist) {
			return nil, nil
		}
		a.logger.Warnf("[FILE] read error: %v", err)
		return nil, err
	}

	om := orderedmap.New()
	if err := sonic.Unmarshal(data, om); err != nil {
		a.logger.Warnf("[FILE] unmarshal to orderedmap failed: %v", err)
		return nil, nil
	}

	var app utils.HarukiSekaiAppInfo
	app.AppVersion = getFirstStr(om, "appVersion", "app_version")
	app.AppHash = getFirstStr(om, "appHash", "app_hash")
	if app.AppVersion == "" {
		a.logger.Warnf("[FILE] missing appVersion in %s", path)
		return nil, nil
	}
	return &app, nil
}

func parseAppInfoFromJSON(body []byte, logger *harukiLogger.Logger) (*utils.HarukiSekaiAppInfo, error) {
	var app utils.HarukiSekaiAppInfo
	if err := sonic.Unmarshal(body, &app); err != nil {
		logger.Warnf("[URL] unmarshal into struct failed: %v", err)
		return nil, nil
	}

	if app.AppVersion == "" || app.AppHash == "" {
		om2 := orderedmap.New()
		if err := sonic.Unmarshal(body, om2); err == nil {
			v := getFirstStr(om2, "appVersion", "app_version")
			h := getFirstStr(om2, "appHash", "app_hash")
			if app.AppVersion == "" {
				app.AppVersion = v
			}
			if app.AppHash == "" {
				app.AppHash = h
			}
		}
	}

	if app.AppVersion == "" {
		return nil, nil
	}
	return &app, nil
}

func (a *HarukiSekaiAppHashUpdater) readAppInfoFromURL(ctx context.Context, source utils.HarukiSekaiAppHashSource, filename string) (*utils.HarukiSekaiAppInfo, error) {
	u := source.URL + "/" + filename
	resp, err := a.client.R().SetContext(ctx).Get(u)
	if err != nil {
		a.logger.Warnf("[URL] request error: %v", err)
		return nil, err
	}
	if resp == nil {
		a.logger.Warnf("[URL] nil response from %s", u)
		return nil, nil
	}
	if !resp.IsSuccess() || len(resp.Body()) == 0 {
		return nil, nil
	}

	return parseAppInfoFromJSON(resp.Body(), a.logger)
}

func (a *HarukiSekaiAppHashUpdater) GetRemoteAppVersion(ctx context.Context, server string, source utils.HarukiSekaiAppHashSource) (*utils.HarukiSekaiAppInfo, error) {
	filename := strings.ToUpper(server) + ".json"

	switch source.Type {
	case utils.HarukiSekaiAppHashSourceTypeFile:
		return a.readAppInfoFromFile(source, filename)
	case utils.HarukiSekaiAppHashSourceTypeUrl:
		return a.readAppInfoFromURL(ctx, source, filename)
	}
	return nil, nil
}

func (a *HarukiSekaiAppHashUpdater) GetLatestRemoteAppInfo(ctx context.Context) (*utils.HarukiSekaiAppInfo, error) {
	var wg sync.WaitGroup
	resultCh := make(chan *utils.HarukiSekaiAppInfo, len(a.sources))

	for _, src := range a.sources {
		wg.Add(1)
		go func(source utils.HarukiSekaiAppHashSource) {
			defer wg.Done()
			app, _ := a.GetRemoteAppVersion(ctx, a.server, source)
			if app == nil {
				return
			}
			if app.AppVersion == "" {
				return
			}
			resultCh <- app
		}(src)
	}

	wg.Wait()
	close(resultCh)

	var latest *utils.HarukiSekaiAppInfo
	for app := range resultCh {
		if app == nil || app.AppVersion == "" {
			continue
		}
		if latest == nil {
			latest = app
			continue
		}
		flag, err := utils.CompareVersion(app.AppVersion, latest.AppVersion)
		if err != nil {
			a.logger.Warnf("Failed to compare versions: %v (a=%s, b=%s)", err, app.AppVersion, latest.AppVersion)
			continue
		}
		if flag {
			latest = app
		}
	}
	return latest, nil
}

func (a *HarukiSekaiAppHashUpdater) GetCurrentAppVersion() (*utils.HarukiSekaiAppInfo, error) {
	b, err := os.ReadFile(*a.serverVersionPath)
	if err != nil {
		return nil, nil
	}
	om := orderedmap.New()
	if err := sonic.Unmarshal(b, om); err != nil {
		return nil, nil
	}
	var app utils.HarukiSekaiAppInfo
	if v, ok := om.Get("appVersion"); ok {
		if s, ok := v.(string); ok {
			app.AppVersion = s
		}
	}
	if v, ok := om.Get("appHash"); ok {
		if s, ok := v.(string); ok {
			app.AppHash = s
		}
	}
	if app.AppVersion == "" && app.AppHash == "" {
		return nil, nil
	}
	return &app, nil
}

func (a *HarukiSekaiAppHashUpdater) SaveNewAppHash(app *utils.HarukiSekaiAppInfo) error {
	path := *a.serverVersionPath
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	om := orderedmap.New()
	if b, err := os.ReadFile(path); err == nil && len(b) > 0 {
		_ = sonic.Unmarshal(b, om)
	} else if err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	om.Set("appVersion", app.AppVersion)
	om.Set("appHash", app.AppHash)
	raw, err := sonic.MarshalIndent(om, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, raw, 0o644)
}

func (a *HarukiSekaiAppHashUpdater) CheckAppVersion() {
	ctx := context.Background()
	local, _ := a.GetCurrentAppVersion()
	remote, _ := a.GetLatestRemoteAppInfo(ctx)
	if local == nil || remote == nil {
		a.logger.Warnf("Local or remote version unavailable")
		return
	}
	flag, err := utils.CompareVersion(remote.AppVersion, local.AppVersion)
	if err != nil {
		a.logger.Warnf("Failed to compare versions: %v", err)
		return
	}
	if flag {
		a.logger.Infof("Found new app version: %s, saving new app hash...", remote.AppVersion)
		if err := a.SaveNewAppHash(remote); err != nil {
			a.logger.Warnf("Failed to save new app hash")
			return
		}
		a.logger.Infof("Saved new app hash")
		return
	}
	a.logger.Infof("No new app version found")
}
