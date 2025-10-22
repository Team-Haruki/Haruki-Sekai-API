package updater

import (
	"context"
	"errors"
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
)

type AppHashUpdater struct {
	sources            []utils.HarukiSekaiAppHashSource
	serverVersionPaths map[utils.HarukiSekaiServerRegion]string
	client             *resty.Client
	logger             *harukiLogger.Logger
}

func NewAppHashUpdater(
	sources []utils.HarukiSekaiAppHashSource,
	jpVersionPath, enVersionPath, twVersionPath, krVersionPath, cnVersionPath *string,
) *AppHashUpdater {
	paths := make(map[utils.HarukiSekaiServerRegion]string)
	if jpVersionPath != nil && *jpVersionPath != "" {
		paths[utils.HarukiSekaiServerRegionJP] = *jpVersionPath
	}
	if enVersionPath != nil && *enVersionPath != "" {
		paths[utils.HarukiSekaiServerRegionEN] = *enVersionPath
	}
	if twVersionPath != nil && *twVersionPath != "" {
		paths[utils.HarukiSekaiServerRegionTW] = *twVersionPath
	}
	if krVersionPath != nil && *krVersionPath != "" {
		paths[utils.HarukiSekaiServerRegionKR] = *krVersionPath
	}
	if cnVersionPath != nil && *cnVersionPath != "" {
		paths[utils.HarukiSekaiServerRegionCN] = *cnVersionPath
	}
	return &AppHashUpdater{
		sources:            sources,
		serverVersionPaths: paths,
		client: func() *resty.Client {
			cli := resty.New()
			cli.SetTimeout(30 * time.Second)
			return cli
		}(),
		logger: harukiLogger.NewLogger("HarukiAppHashUpdater", "INFO", nil),
	}
}

func (a *AppHashUpdater) GetRemoteAppVersion(ctx context.Context, server utils.HarukiSekaiServerRegion, source utils.HarukiSekaiAppHashSource) (*utils.HarukiSekaiAppInfo, error) {
	filename := strings.ToUpper(string(server)) + ".json"

	switch source.Type {
	case utils.HarukiSekaiAppHashSourceTypeFile:
		path := filepath.Join(source.Dir, filename)
		data, err := os.ReadFile(path)
		if err != nil {
			if errors.Is(err, fs.ErrNotExist) {
				return nil, nil
			}
			return nil, err
		}
		var app utils.HarukiSekaiAppInfo
		if err := sonic.Unmarshal(data, &app); err != nil {
			return nil, nil
		}
		return &app, nil

	case utils.HarukiSekaiAppHashSourceTypeUrl:
		url := source.URL + filename
		resp, err := a.client.R().SetContext(ctx).Get(url)
		if err != nil {
			return nil, err
		}
		if !resp.IsSuccess() {
			return nil, nil
		}
		var app utils.HarukiSekaiAppInfo
		if err := sonic.Unmarshal(resp.Body(), &app); err != nil {
			return nil, nil
		}
		return &app, nil
	}
	return nil, nil
}

func (a *AppHashUpdater) GetLatestRemoteAppInfo(ctx context.Context, server utils.HarukiSekaiServerRegion) (*utils.HarukiSekaiAppInfo, error) {
	var wg sync.WaitGroup
	resultCh := make(chan *utils.HarukiSekaiAppInfo, len(a.sources))

	for _, src := range a.sources {
		wg.Add(1)
		go func(source utils.HarukiSekaiAppHashSource) {
			defer wg.Done()
			app, _ := a.GetRemoteAppVersion(ctx, server, source)
			if app != nil {
				resultCh <- app
			}
		}(src)
	}

	wg.Wait()
	close(resultCh)

	var latest *utils.HarukiSekaiAppInfo
	for app := range resultCh {
		if latest == nil {
			latest = app
			continue
		}
		flag, err := utils.CompareVersion(app.AppVersion, latest.AppVersion)
		if err != nil {
			a.logger.Warnf("%s server: failed to compare versions: %v", server, err)
			continue
		}
		if flag {
			latest = app
		}
	}
	return latest, nil
}

func (a *AppHashUpdater) GetCurrentAppVersion(server utils.HarukiSekaiServerRegion) (*utils.HarukiSekaiAppInfo, error) {
	path := a.serverVersionPaths[server]
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, nil
	}
	var app utils.HarukiSekaiAppInfo
	if err := sonic.Unmarshal(data, &app); err != nil {
		return nil, nil
	}
	return &app, nil
}

func (a *AppHashUpdater) SaveNewAppHash(server utils.HarukiSekaiServerRegion, app *utils.HarukiSekaiAppInfo) error {
	path := a.serverVersionPaths[server]
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	var existing map[string]any
	if b, err := os.ReadFile(path); err == nil && len(b) > 0 {
		_ = sonic.Unmarshal(b, &existing)
	} else if err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	if existing == nil {
		existing = make(map[string]any)
	}
	existing["appVersion"] = app.AppVersion
	existing["appHash"] = app.AppHash
	raw, err := sonic.MarshalIndent(existing, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, raw, 0o644)
}

func (a *AppHashUpdater) CheckAppVersion(ctx context.Context, server utils.HarukiSekaiServerRegion) (bool, error) {
	local, _ := a.GetCurrentAppVersion(server)
	remote, _ := a.GetLatestRemoteAppInfo(ctx, server)
	if local == nil || remote == nil {
		a.logger.Warnf("%s server: local or remote version unavailable", server)
		return false, nil
	}
	flag, err := utils.CompareVersion(remote.AppVersion, local.AppVersion)
	if err != nil {
		a.logger.Warnf("%s server: failed to compare versions: %v", server, err)
		return false, err
	}
	if flag {
		a.logger.Infof("%s server found new app version: %s, saving new app hash...", server, remote.AppVersion)
		if err := a.SaveNewAppHash(server, remote); err != nil {
			a.logger.Warnf("%s server failed to save new app hash", server)
			return false, err
		}
		a.logger.Infof("%s server saved new app hash", server)
		return true, nil
	}
	a.logger.Infof("%s server no new app version found", server)
	return false, nil
}

func (a *AppHashUpdater) CheckAppVersionConcurrently(ctx context.Context) {
	var wg sync.WaitGroup
	for server := range a.serverVersionPaths {
		wg.Add(1)
		go func(s utils.HarukiSekaiServerRegion) {
			defer wg.Done()
			if _, err := a.CheckAppVersion(ctx, s); err != nil {
				a.logger.Warnf("%s server: failed to check app version: %v", s, err)
			}
		}(server)
	}
	wg.Wait()
}
