package updater

import (
	"context"
	"encoding/json"
	"errors"
	harukiLogger "haruki-sekai-api/logger"
	"io/fs"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"haruki-sekai-api/utils"
)

type AppHashUpdater struct {
	sources           []utils.HarukiSekaiAppHashSource
	serverVersionDirs map[utils.HarukiSekaiServerRegion]string
	client            *http.Client
	logger            *harukiLogger.Logger
}

func NewAppHashUpdater(
	sources []utils.HarukiSekaiAppHashSource,
	serverVersionDirs map[utils.HarukiSekaiServerRegion]string,
) *AppHashUpdater {
	return &AppHashUpdater{
		sources:           sources,
		serverVersionDirs: serverVersionDirs,
		client: &http.Client{
			Timeout: 15 * time.Second,
		},
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
		if err := json.Unmarshal(data, &app); err != nil {
			return nil, nil
		}
		return &app, nil

	case utils.HarukiSekaiAppHashSourceTypeUrl:
		url := source.URL + filename
		req, err := http.NewRequestWithContext(ctx, "GET", url, nil)
		if err != nil {
			return nil, err
		}
		resp, err := a.client.Do(req)
		if err != nil {
			return nil, err
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			return nil, nil
		}
		var app utils.HarukiSekaiAppInfo
		if err := json.NewDecoder(resp.Body).Decode(&app); err != nil {
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
	path := filepath.Join(a.serverVersionDirs[server], "current_version.json")
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, nil
	}
	var app utils.HarukiSekaiAppInfo
	if err := json.Unmarshal(data, &app); err != nil {
		return nil, nil
	}
	return &app, nil
}

func (a *AppHashUpdater) SaveNewAppHash(server utils.HarukiSekaiServerRegion, app *utils.HarukiSekaiAppInfo) error {
	path := filepath.Join(a.serverVersionDirs[server], "current_version.json")
	data := map[string]string{
		"appVersion": app.AppVersion,
		"appHash":    app.AppHash,
	}
	raw, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, raw, 0644)
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
	for server := range a.serverVersionDirs {
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
