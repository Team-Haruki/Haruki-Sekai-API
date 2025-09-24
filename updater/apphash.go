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
	sources           []utils.HarukiAppHashSource
	serverVersionDirs map[utils.SekaiRegion]string
	client            *http.Client
	logger            *harukiLogger.Logger
}

func NewAppHashUpdater(
	sources []utils.HarukiAppHashSource,
	serverVersionDirs map[utils.SekaiRegion]string,
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

func (a *AppHashUpdater) GetRemoteAppVersion(ctx context.Context, server utils.SekaiRegion, source utils.HarukiAppHashSource) (*utils.HarukiAppInfo, error) {
	filename := strings.ToUpper(string(server)) + ".json"

	switch source.Type {
	case utils.HarukiAppHashSourceTypeFile:
		path := filepath.Join(source.Dir, filename)
		data, err := os.ReadFile(path)
		if err != nil {
			if errors.Is(err, fs.ErrNotExist) {
				return nil, nil
			}
			return nil, err
		}
		var app utils.HarukiAppInfo
		if err := json.Unmarshal(data, &app); err != nil {
			return nil, nil
		}
		return &app, nil

	case utils.HarukiAppHashSourceTypeUrl:
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
		var app utils.HarukiAppInfo
		if err := json.NewDecoder(resp.Body).Decode(&app); err != nil {
			return nil, nil
		}
		return &app, nil
	}
	return nil, nil
}

func (a *AppHashUpdater) GetLatestRemoteAppInfo(ctx context.Context, server utils.SekaiRegion) (*utils.HarukiAppInfo, error) {
	var wg sync.WaitGroup
	resultCh := make(chan *utils.HarukiAppInfo, len(a.sources))

	for _, src := range a.sources {
		wg.Add(1)
		go func(source utils.HarukiAppHashSource) {
			defer wg.Done()
			app, _ := a.GetRemoteAppVersion(ctx, server, source)
			if app != nil {
				resultCh <- app
			}
		}(src)
	}

	wg.Wait()
	close(resultCh)

	var latest *utils.HarukiAppInfo
	for app := range resultCh {
		if latest == nil || CompareVersion(app.AppVersion, latest.AppVersion) {
			latest = app
		}
	}
	return latest, nil
}

func (a *AppHashUpdater) GetCurrentAppVersion(server utils.SekaiRegion) (*utils.HarukiAppInfo, error) {
	path := filepath.Join(a.serverVersionDirs[server], "current_version.json")
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, nil
	}
	var app utils.HarukiAppInfo
	if err := json.Unmarshal(data, &app); err != nil {
		return nil, nil
	}
	return &app, nil
}

func (a *AppHashUpdater) SaveNewAppHash(server utils.SekaiRegion, app *utils.HarukiAppInfo) error {
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

func (a *AppHashUpdater) CheckAppVersion(ctx context.Context, server utils.SekaiRegion) (bool, error) {
	local, _ := a.GetCurrentAppVersion(server)
	remote, _ := a.GetLatestRemoteAppInfo(ctx, server)
	if local == nil || remote == nil {
		a.logger.Warnf("%s server: local or remote version unavailable", server)
		return false, nil
	}
	if CompareVersion(remote.AppVersion, local.AppVersion) {
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
		go func(s utils.SekaiRegion) {
			defer wg.Done()
			a.CheckAppVersion(ctx, s)
		}(server)
	}
	wg.Wait()
}
