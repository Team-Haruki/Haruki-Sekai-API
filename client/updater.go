package client

import (
	"context"
	"fmt"
	"haruki-sekai-api/config"
	"haruki-sekai-api/utils"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/bytedance/sonic"
	"github.com/go-git/go-git/v5"
	"github.com/go-resty/resty/v2"
)

func (mgr *SekaiClientManager) loadVersionFile() (map[string]any, error) {
	data, err := os.ReadFile(mgr.ServerConfig.VersionPath)
	if err != nil {
		return nil, err
	}
	var result map[string]any
	if err := sonic.Unmarshal(data, &result); err != nil {
		return nil, err
	}
	return result, nil
}

func (mgr *SekaiClientManager) saveFile(filePath string, data any) error {
	jsonData, err := sonic.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}
	dir := filepath.Dir(filePath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}
	return os.WriteFile(filePath, jsonData, 0644)
}

func (mgr *SekaiClientManager) callHarukiAssetUpdater(updaterInfo utils.HarukiAssetUpdaterInfo, payload HarukiSekaiAssetUpdaterPayload) {
	endpoint := updaterInfo.URL
	cli := resty.New()
	cli.SetTimeout(30 * time.Second)

	for {
		req := cli.R().
			SetHeader("Content-Type", "application/json").
			SetHeader("User-Agent", fmt.Sprintf("Haruki-Sekai-API/%s", config.Version)).
			SetBody(payload)
		if updaterInfo.Authorization != "" {
			req.SetHeader("Authorization", "Bearer "+updaterInfo.Authorization)
		}
		resp, err := req.Post(endpoint)
		if err != nil {
			return
		}
		if resp.StatusCode() == 409 {
			time.Sleep(1 * time.Minute)
			continue
		}
		return
	}
}

func (mgr *SekaiClientManager) callAllHarukiAssetUpdater(assetVersion, assetHash string) {
	if len(mgr.AssetUpdaterServers) == 0 {
		return
	}
	var payload = HarukiSekaiAssetUpdaterPayload{Server: mgr.Server, AssetVersion: assetVersion, AssetHash: assetHash}
	var wg sync.WaitGroup
	for _, info := range mgr.AssetUpdaterServers {
		if info == nil {
			continue
		}
		wg.Add(1)
		go func(u *utils.HarukiAssetUpdaterInfo) {
			defer wg.Done()
			mgr.callHarukiAssetUpdater(*u, payload)
		}(info)
	}
	wg.Wait()
}

func (mgr *SekaiClientManager) CheckSekaiMasterUpdate() {
	ctx := context.Background()
	var requireUpdateMasterData = false
	var requireUpdateAsset = false
	var currentServerCDNVersion int
	var splitMasterDataList []string

	currentLocalVersion, err := mgr.loadVersionFile()
	if err != nil {
		mgr.Logger.Errorf("Sekai updater failed to load version file: %v", err)
		return
	}
	sekaiClient := mgr.getClient()
	if sekaiClient == nil {
		mgr.Logger.Errorf("Sekai updater failed to initialize client, skipped.")
		return
	}
	respAny, err := sekaiClient.Login(ctx)
	if err != nil {
		mgr.Logger.Errorf("Sekai updater failed to login: %v", err)
		return
	}

	var currentServerVersion map[string]any
	if m, ok := respAny.(map[string]any); ok {
		currentServerVersion = m
	} else {
		if raw, err2 := sonic.Marshal(respAny); err2 == nil {
			_ = sonic.Unmarshal(raw, &currentServerVersion)
		}
	}
	if currentServerVersion == nil {
		mgr.Logger.Errorf("Sekai updater found unexpected login response type: %T", respAny)
		return
	}

	currentServerDataVersion := utils.GetString(currentServerVersion, "dataVersion")
	currentServerAssetVersion := utils.GetString(currentServerVersion, "assetVersion")
	currentServerAssetHash := utils.GetString(currentServerVersion, "assetHash")
	if mgr.Server == utils.HarukiSekaiServerRegionJP || mgr.Server == utils.HarukiSekaiServerRegionEN {
		currentLocalDataVersion := utils.GetString(currentLocalVersion, "dataVersion")
		currentLocalAssetVersion := utils.GetString(currentLocalVersion, "assetVersion")
		isNewer, err := utils.CompareVersion(currentServerDataVersion, currentLocalDataVersion)
		if err != nil {
			mgr.Logger.Warnf("Sekai updater failed to compare data version: %v", err)
			return
		} else if isNewer {
			mgr.Logger.Criticalf("Sekai updater found new master data version: %s", currentServerDataVersion)
			if arr, ok := currentServerVersion["suiteMasterSplitPath"].([]string); ok {
				splitMasterDataList = arr
			} else {
				mgr.Logger.Warnf("Sekai updater found unexpected suiteMasterSplitPath type: %T", currentServerVersion["suiteMasterSplitPath"])
			}
			requireUpdateMasterData = true
		}
		isNewer, err = utils.CompareVersion(currentServerAssetVersion, currentLocalAssetVersion)
		if err != nil {
			mgr.Logger.Warnf("Sekai updater failed to compare asset version: %v", err)
			return
		} else if isNewer {
			mgr.Logger.Criticalf("Sekai updater found new asset version: %s", currentServerAssetVersion)
			requireUpdateAsset = true
		}
	} else {
		currentLocalCDNVersion := utils.GetInt(currentLocalVersion, "cdnVersion")
		currentServerCDNVersion = utils.GetInt(currentServerVersion, "cdnVersion")
		if currentLocalCDNVersion < currentServerCDNVersion {
			mgr.Logger.Criticalf("Sekai updater found new cdn version: %d", currentServerCDNVersion)
			requireUpdateMasterData = true
			requireUpdateAsset = true
		}
	}

	if requireUpdateAsset {
		go mgr.callAllHarukiAssetUpdater(currentServerAssetVersion, currentServerAssetHash)
	}

	if requireUpdateMasterData {
		go mgr.updateMasterData(currentServerDataVersion, splitMasterDataList, currentServerCDNVersion)
	}

	if requireUpdateMasterData || requireUpdateAsset {
		currentLocalVersion["dataVersion"] = currentServerDataVersion
		currentLocalVersion["assetVersion"] = currentServerAssetVersion
		currentLocalVersion["assetHash"] = currentServerAssetHash

		if mgr.Server != utils.HarukiSekaiServerRegionJP && mgr.Server != utils.HarukiSekaiServerRegionEN {
			currentLocalVersion["cdnVersion"] = currentServerCDNVersion
		}

		if err := mgr.saveFile(mgr.ServerConfig.VersionPath, currentLocalVersion); err != nil {
			mgr.Logger.Errorf("Sekai updater failed to save version file: %v", err)
			return
		}

		versionDir := filepath.Dir(mgr.ServerConfig.VersionPath)
		versionedFile := filepath.Join(versionDir, currentServerDataVersion+".json")
		if err := mgr.saveFile(versionedFile, currentLocalVersion); err != nil {
			mgr.Logger.Errorf("Sekai updater failed to save version file: %v", err)
			return
		}
	}

}

func (mgr *SekaiClientManager) saveSplitMasterData(master map[string]any) {
	mgr.Logger.Infof("Sekai updater saving split master data...")
	if err := os.MkdirAll(mgr.ServerConfig.MasterDir, 0755); err != nil {
		mgr.Logger.Errorf("Sekai updater failed to create master data directory: %v", err)
	}

	var wg sync.WaitGroup
	errChan := make(chan error, len(master))

	for key, value := range master {
		wg.Add(1)
		go func(k string, v any) {
			defer wg.Done()
			filePath := filepath.Join(mgr.ServerConfig.MasterDir, k+".json")
			if err := mgr.saveFile(filePath, v); err != nil {
				errChan <- fmt.Errorf("failed to save %s: %v", k, err)
			}
		}(key, value)
	}

	wg.Wait()
	close(errChan)

	for err := range errChan {
		if err != nil {
			mgr.Logger.Errorf("Sekai updater failed to save split master data: %v", err)
		}
	}

	mgr.Logger.Infof("Sekai updater saved split master data.")
	return
}

func (mgr *SekaiClientManager) updateMasterData(dataVersion string, paths []string, cdnVersion int) {
	mgr.Logger.Infof("Sekai updater downloading new master data...")
	var err error
	masterData := make(map[string]any)
	sekaiClient := mgr.getClient()
	if sekaiClient == nil {
		mgr.Logger.Errorf("Sekai updater failed to initialize client, skipped.")
		return
	}

	if mgr.Server == utils.HarukiSekaiServerRegionJP || mgr.Server == utils.HarukiSekaiServerRegionEN {
		masterData, err = sekaiClient.GetCPMasterData(paths)
		if err != nil {
			mgr.Logger.Errorf("Sekai updater failed to get master data: %v", err)
			return
		}
	} else {
		masterData, err = sekaiClient.GetNuverseMasterData(cdnVersion)
		if err != nil {
			mgr.Logger.Errorf("Sekai updater failed to get master data: %v", err)
			return
		}

	}

	mgr.Logger.Infof("Sekai updater downloaded new master data.")
	mgr.saveSplitMasterData(masterData)
	repoRoot := filepath.Dir(mgr.ServerConfig.MasterDir)
	repo, err := git.PlainOpen(repoRoot)
	if err != nil {
		mgr.Logger.Errorf("Sekai updater failed to open git repo at %s: %v", repoRoot, err)
		return
	}
	if mgr.Git != nil {
		if err := mgr.Git.PushRemote(repo, dataVersion); err != nil {
			mgr.Logger.Errorf("Sekai updater failed to push repo: %v", err)
			return
		}
		mgr.Logger.Infof("Sekai updater pushed changes to remote with data version %s", dataVersion)
	} else {
		mgr.Logger.Warnf("Sekai updater Git is not configured, skipped pushing to remote repo.")
	}
}
