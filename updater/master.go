package updater

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"haruki-sekai-api/client"
	"haruki-sekai-api/utils"
	"haruki-sekai-api/utils/logger"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"sync"
	"time"

	"github.com/bytedance/sonic"
)

type SekaiMasterUpdater struct {
	servers             map[utils.HarukiSekaiServerRegion]client.SekaiServerInfo
	managers            map[utils.HarukiSekaiServerRegion]*client.SekaiClientManager
	masterDirs          map[utils.HarukiSekaiServerRegion]string
	versionDirs         map[utils.HarukiSekaiServerRegion]string
	assetUpdaterServers []client.HarukiAssetUpdaterInfo
	logger              *logger.Logger
}

func NewSekaiMasterUpdater(
	servers map[utils.HarukiSekaiServerRegion]client.SekaiServerInfo,
	managers map[utils.HarukiSekaiServerRegion]*client.SekaiClientManager,
	masterDirs map[utils.HarukiSekaiServerRegion]string,
	versionDirs map[utils.HarukiSekaiServerRegion]string,
	assetUpdaterServers []client.HarukiAssetUpdaterInfo,
) *SekaiMasterUpdater {
	return &SekaiMasterUpdater{
		servers:             servers,
		managers:            managers,
		masterDirs:          masterDirs,
		versionDirs:         versionDirs,
		assetUpdaterServers: assetUpdaterServers,
		logger:              logger.NewLogger("SekaiMasterUpdater", "DEBUG", nil),
	}
}

func (s *SekaiMasterUpdater) loadFile(filePath string) (map[string]any, error) {
	data, err := os.ReadFile(filePath)
	if err != nil {
		return nil, err
	}

	var result map[string]any
	if err := sonic.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return result, nil
}

func (s *SekaiMasterUpdater) saveFile(filePath string, data any) error {
	jsonData, err := sonic.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}

	// 确保目录存在
	dir := filepath.Dir(filePath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	return os.WriteFile(filePath, jsonData, 0644)
}

// callAssetUpdater 调用 Haruki Sekai Asset Updater
func (s *SekaiMasterUpdater) callAssetUpdater(ctx context.Context, options map[string]any) (*http.Response, error) {
	cli := &http.Client{Timeout: 30 * time.Second}

	url, ok := options["url"].(string)
	if !ok {
		return nil, fmt.Errorf("invalid url")
	}

	method, ok := options["method"].(string)
	if !ok {
		return nil, fmt.Errorf("invalid method")
	}

	jsonBody := options["json"]
	bodyBytes, err := json.Marshal(jsonBody)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequestWithContext(ctx, method, url, bytes.NewBuffer(bodyBytes))
	if err != nil {
		return nil, err
	}

	req.Header.Set("Content-Type", "application/json")

	if headers, ok := options["headers"].(map[string]string); ok {
		for k, v := range headers {
			req.Header.Set(k, v)
		}
	}

	resp, err := cli.Do(req)
	if err != nil {
		return nil, err
	}

	switch resp.StatusCode {
	case 200: // SekaiApiHttpStatusOk
		return resp, nil
	case 409: // SekaiApiHttpStatusConflict
		resp.Body.Close()
		time.Sleep(60 * time.Second)
		return s.callAssetUpdater(ctx, options)
	default:
		resp.Body.Close()
		return nil, fmt.Errorf("unexpected status code: %d", resp.StatusCode)
	}
}

func (s *SekaiMasterUpdater) assetUpdater(ctx context.Context, server utils.HarukiSekaiServerRegion, data map[string]any) {
	body := map[string]any{
		"server": string(server),
	}

	if assetVersion, ok := data["assetVersion"]; ok {
		body["assetVersion"] = assetVersion
	}

	if assetHash, ok := data["assetHash"]; ok {
		body["assetHash"] = assetHash
	}

	var wg sync.WaitGroup
	for _, updater := range s.assetUpdaterServers {
		wg.Add(1)
		go func(upd client.HarukiAssetUpdaterInfo) {
			defer wg.Done()

			options := map[string]any{
				"url":    upd.Url + "/update_asset",
				"method": "POST",
				"json":   body,
				"headers": map[string]string{
					"User-Agent": "Haruki Sekai API/v3.0.0",
				},
			}

			if upd.Authorization != "" {
				headers := options["headers"].(map[string]string)
				headers["Authorization"] = "Bearer " + upd.Authorization
			}

			resp, err := s.callAssetUpdater(ctx, options)
			if err != nil {
				s.logger.Errorf("Failed to call asset updater: %v", err)
				return
			}
			if resp != nil {
				resp.Body.Close()
			}
		}(updater)
	}
	wg.Wait()
}

// checkUpdate 检查更新
func (s *SekaiMasterUpdater) checkUpdate(ctx context.Context, server utils.HarukiSekaiServerRegion, manager *client.SekaiClientManager) (map[utils.HarukiSekaiServerRegion]string, error) {
	updateMaster := false
	updateAsset := false

	versionFile := filepath.Join(s.versionDirs[server], "current_version.json")
	currentVersion, err := s.loadFile(versionFile)
	if err != nil {
		return nil, fmt.Errorf("failed to load version file: %v", err)
	}

	currentServerVersion, err := manager.GetLoginData()
	if err != nil {
		return nil, fmt.Errorf("failed to get login data: %v", err)
	}

	currentDataVersion := getString(currentVersion, "dataVersion")
	currentAssetVersion := getString(currentVersion, "assetVersion")
	currentServerDataVersion := getString(currentServerVersion, "dataVersion")
	currentServerAssetVersion := getString(currentServerVersion, "assetVersion")
	currentServerAssetHash := getString(currentServerVersion, "assetHash")

	if server == utils.HarukiSekaiServerRegionJP || server == utils.HarukiSekaiServerRegionEN {
		// 检查数据版本更新
		isNewer, err := utils.CompareVersion(currentServerDataVersion, currentDataVersion)
		if err != nil {
			s.logger.Warnf("Failed to compare data version: %v", err)
		} else if isNewer {
			s.logger.Criticalf("%s server found new master data version: %s",
				string(server), currentServerDataVersion)
			updateMaster = true
		}

		// 检查资源版本更新
		isNewer, err = utils.CompareVersion(currentServerAssetVersion, currentAssetVersion)
		if err != nil {
			s.logger.Warnf("Failed to compare asset version: %v", err)
		} else if isNewer {
			s.logger.Criticalf("%s server found new asset version: %s",
				string(server), currentServerAssetVersion)
			updateAsset = true
		}
	} else {
		// 对于其他服务器，检查 CDN 版本
		currentCdnVersion := getInt(currentVersion, "cdnVersion")
		currentServerCdnVersion := getInt(currentServerVersion, "cdnVersion")

		if currentCdnVersion < currentServerCdnVersion {
			s.logger.Criticalf("%s server found new cdn version: %d",
				string(server), currentServerCdnVersion)
			updateMaster = true
			updateAsset = true
		}
	}

	if updateAsset {
		s.assetUpdater(ctx, server, currentServerVersion)
	}

	if updateMaster {
		if err := s.updateMaster(ctx, server, manager); err != nil {
			return nil, fmt.Errorf("failed to update master: %v", err)
		}
	}

	if updateAsset || updateMaster {
		// 更新版本文件
		currentVersion["dataVersion"] = currentServerDataVersion
		currentVersion["assetVersion"] = currentServerAssetVersion
		currentVersion["assetHash"] = currentServerAssetHash

		if cdnVersion, ok := currentServerVersion["cdnVersion"]; ok {
			currentVersion["cdnVersion"] = cdnVersion
		}

		if err := s.saveFile(versionFile, currentVersion); err != nil {
			return nil, fmt.Errorf("failed to save version file: %v", err)
		}

		versionedFile := filepath.Join(s.versionDirs[server], currentServerDataVersion+".json")
		if err := s.saveFile(versionedFile, currentVersion); err != nil {
			return nil, fmt.Errorf("failed to save versioned file: %v", err)
		}

		return map[utils.HarukiSekaiServerRegion]string{server: currentServerDataVersion}, nil
	}

	return nil, nil
}

// checkUpdateConcurrently 并发检查更新
func (s *SekaiMasterUpdater) CheckUpdateConcurrently(ctx context.Context) (map[utils.HarukiSekaiServerRegion]string, error) {
	s.logger.Infof("Starting concurrent update check...")

	type result struct {
		data map[utils.HarukiSekaiServerRegion]string
		err  error
	}

	resultChan := make(chan result, len(s.managers))
	var wg sync.WaitGroup

	for server, manager := range s.managers {
		wg.Add(1)
		go func(srv utils.HarukiSekaiServerRegion, mgr *client.SekaiClientManager) {
			defer wg.Done()
			data, err := s.checkUpdate(ctx, srv, mgr)
			resultChan <- result{data: data, err: err}
		}(server, manager)
	}

	wg.Wait()
	close(resultChan)

	resultDict := make(map[utils.HarukiSekaiServerRegion]string)
	for res := range resultChan {
		if res.err != nil {
			s.logger.Errorf("Update check error: %v", res.err)
			continue
		}
		if res.data != nil {
			for k, v := range res.data {
				resultDict[k] = v
			}
		}
	}

	if len(resultDict) > 0 {
		return resultDict, nil
	}

	s.logger.Infof("Update check completed, no updates found")
	return nil, nil
}

func (s *SekaiMasterUpdater) saveSplitMasterData(server utils.HarukiSekaiServerRegion, master map[string]any) error {
	s.logger.Infof("Saving %s server split master data...", string(server))

	masterDir := s.masterDirs[server]
	if err := os.MkdirAll(masterDir, 0755); err != nil {
		return fmt.Errorf("failed to create master directory: %v", err)
	}

	var wg sync.WaitGroup
	errChan := make(chan error, len(master))

	for key, value := range master {
		wg.Add(1)
		go func(k string, v any) {
			defer wg.Done()
			filePath := filepath.Join(masterDir, k+".json")
			if err := s.saveFile(filePath, v); err != nil {
				errChan <- fmt.Errorf("failed to save %s: %v", k, err)
			}
		}(key, value)
	}

	wg.Wait()
	close(errChan)

	for err := range errChan {
		if err != nil {
			return err
		}
	}

	s.logger.Infof("Saved %s server split master data", string(server))
	return nil
}

func (s *SekaiMasterUpdater) updateMaster(ctx context.Context, server utils.HarukiSekaiServerRegion, manager *client.SekaiClientManager) error {
	s.logger.Infof("Downloading %s new master data...", string(server))

	masterData, err := manager.DownloadMaster()
	if err != nil {
		return fmt.Errorf("failed to download master data: %v", err)
	}

	s.logger.Infof("Downloaded %s new master data", string(server))

	return s.saveSplitMasterData(server, masterData)
}

// 辅助函数
func getString(m map[string]any, key string) string {
	if v, ok := m[key]; ok {
		if s, ok := v.(string); ok {
			return s
		}
	}
	return ""
}

func getInt(m map[string]any, key string) int {
	if v, ok := m[key]; ok {
		switch val := v.(type) {
		case int:
			return val
		case float64:
			return int(val)
		case string:
			if i, err := strconv.Atoi(val); err == nil {
				return i
			}
		}
	}
	return 0
}
