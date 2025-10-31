package client

import (
	"context"
	"fmt"
	"haruki-sekai-api/config"
	"haruki-sekai-api/utils"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/bytedance/sonic"
	"github.com/go-git/go-git/v6"
	"github.com/go-resty/resty/v2"
	"github.com/iancoleman/orderedmap"
)

func (mgr *SekaiClientManager) loadVersionFile() (*orderedmap.OrderedMap, error) {
	data, err := os.ReadFile(mgr.ServerConfig.VersionPath)
	if err != nil {
		return nil, err
	}
	om := orderedmap.New()
	if err := sonic.Unmarshal(data, om); err != nil {
		return nil, err
	}
	return om, nil
}

func (mgr *SekaiClientManager) saveFile(filePath string, data any) error {
	dir := filepath.Dir(filePath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	file, err := os.OpenFile(filePath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0644)
	if err != nil {
		return err
	}
	defer func(file *os.File) {
		_ = file.Close()
	}(file)

	json := sonic.Config{EscapeHTML: false}.Froze()
	jsonData, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}

	_, err = file.Write(jsonData)
	jsonData = nil
	return err
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
		wg.Add(1)
		go func(u utils.HarukiAssetUpdaterInfo) {
			defer wg.Done()
			mgr.callHarukiAssetUpdater(u, payload)
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
	loginResponse, err := sekaiClient.Login(ctx)
	if err != nil {
		mgr.Logger.Errorf("Sekai updater failed to login: %v", err)
		return
	}

	currentServerDataVersion := loginResponse.DataVersion
	currentServerAssetVersion := loginResponse.AssetVersion
	currentServerAssetHash := loginResponse.AssetHash
	if mgr.Server == utils.HarukiSekaiServerRegionJP || mgr.Server == utils.HarukiSekaiServerRegionEN {
		currentLocalDataVersion := utils.GetString(currentLocalVersion, "dataVersion")
		currentLocalAssetVersion := utils.GetString(currentLocalVersion, "assetVersion")
		isNewer, err := utils.CompareVersion(currentServerDataVersion, currentLocalDataVersion)
		if err != nil {
			mgr.Logger.Warnf("Sekai updater failed to compare data version: %v", err)
			return
		} else if isNewer {
			mgr.Logger.Criticalf("Sekai updater found new master data version: %s", currentServerDataVersion)
			if loginResponse.SuiteMasterSplitPath != nil {
				splitMasterDataList = loginResponse.SuiteMasterSplitPath
			} else {
				mgr.Logger.Warnf("Sekai updater can not found suiteMasterSplitPath")
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
		currentServerCDNVersion = loginResponse.CDNVersion
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
		currentLocalVersion.Set("dataVersion", currentServerDataVersion)
		currentLocalVersion.Set("assetVersion", currentServerAssetVersion)
		currentLocalVersion.Set("assetHash", currentServerAssetHash)

		if mgr.Server != utils.HarukiSekaiServerRegionJP && mgr.Server != utils.HarukiSekaiServerRegionEN {
			currentLocalVersion.Set("cdnVersion", currentServerCDNVersion)
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

func (mgr *SekaiClientManager) saveSplitMasterData(master *orderedmap.OrderedMap) {
	mgr.Logger.Infof("Sekai updater saving split master data...")
	if err := os.MkdirAll(mgr.ServerConfig.MasterDir, 0755); err != nil {
		mgr.Logger.Errorf("Sekai updater failed to create master data directory: %v", err)
	}

	var wg sync.WaitGroup
	errChan := make(chan error, len(master.Keys()))
	sem := make(chan struct{}, 5)

	for _, key := range master.Keys() {
		value, _ := master.Get(key)
		wg.Add(1)
		go func(k string, v any) {
			defer wg.Done()
			sem <- struct{}{}
			defer func() { <-sem }()

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
	sekaiClient := mgr.getClient()
	if sekaiClient == nil {
		mgr.Logger.Errorf("Sekai updater failed to initialize client, skipped.")
		return
	}

	var err error
	if mgr.Server == utils.HarukiSekaiServerRegionJP || mgr.Server == utils.HarukiSekaiServerRegionEN {
		err = mgr.streamCPMasterData(sekaiClient, paths)
		if err != nil {
			mgr.Logger.Errorf("Sekai updater failed to get master data: %v", err)
			return
		}
	} else {
		err = mgr.streamNuverseMasterData(sekaiClient, cdnVersion)
		if err != nil {
			mgr.Logger.Errorf("Sekai updater failed to get master data: %v", err)
			return
		}
	}

	mgr.Logger.Infof("Sekai updater saved new master data.")
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

	runtime.GC()
}

func (mgr *SekaiClientManager) streamCPMasterData(client *SekaiClient, paths []string) error {
	if err := os.MkdirAll(mgr.ServerConfig.MasterDir, 0755); err != nil {
		return fmt.Errorf("failed to create master data directory: %w", err)
	}
	ctx := context.Background()
	var allErrors []error
	var errorsMu sync.Mutex
	var pathWg sync.WaitGroup
	pathSem := make(chan struct{}, 2)
	for _, rawPath := range paths {
		if rawPath == "" {
			continue
		}
		pathWg.Add(1)
		go func(rp string) {
			defer pathWg.Done()
			pathSem <- struct{}{}
			defer func() { <-pathSem }()
			p := rp
			if !strings.HasPrefix(p, "/") {
				p = "/" + p
			}
			resp, err := client.Get(ctx, p, nil)
			if err != nil {
				errorsMu.Lock()
				allErrors = append(allErrors, fmt.Errorf("failed to get %s: %w", rp, err))
				errorsMu.Unlock()
				return
			}
			body := resp.Body()
			om, err := client.Cryptor.UnpackOrdered(body)
			body = nil
			if err != nil {
				errorsMu.Lock()
				allErrors = append(allErrors, fmt.Errorf("unpack master part failed: path=%s, err=%w", rp, err))
				errorsMu.Unlock()
				return
			}
			if om == nil {
				errorsMu.Lock()
				allErrors = append(allErrors, fmt.Errorf("unexpected master data: nil ordered map at path %s", rp))
				errorsMu.Unlock()
				return
			}
			keys := om.Keys()
			var processedFiles sync.Map
			var fileWg sync.WaitGroup
			fileSem := make(chan struct{}, 2)
			var hasError bool
			var savedCount int32
			for _, k := range keys {
				if _, loaded := processedFiles.LoadOrStore(k, true); loaded {
					continue
				}
				v, ok := om.Get(k)
				if !ok {
					mgr.Logger.Warnf("Could not get value for file %s from path %s", k, rp)
					continue
				}
				fileWg.Add(1)
				go func(key string, value any) {
					defer fileWg.Done()
					fileSem <- struct{}{}
					defer func() {
						<-fileSem
						value = nil
					}()
					filePath := filepath.Join(mgr.ServerConfig.MasterDir, key+".json")
					saveErr := mgr.saveFile(filePath, value)
					if saveErr != nil {
						mgr.Logger.Errorf("Failed to save %s from path %s: %v", key, rp, saveErr)
						errorsMu.Lock()
						allErrors = append(allErrors, fmt.Errorf("failed to save %s from path %s: %w", key, rp, saveErr))
						errorsMu.Unlock()
						hasError = true
					} else {
						atomic.AddInt32(&savedCount, 1)
					}
				}(k, v)
				v = nil
			}
			fileWg.Wait()
			om = nil
			if hasError {
				mgr.Logger.Warnf("Processed path %s with errors: saved %d/%d files", rp, savedCount, len(keys))
			}
			runtime.GC()
		}(rawPath)
	}
	pathWg.Wait()
	if len(allErrors) > 0 {
		mgr.Logger.Errorf("Encountered %d errors while processing master data", len(allErrors))
		for i, err := range allErrors {
			if i < 10 {
				mgr.Logger.Errorf("Error %d: %v", i+1, err)
			}
		}
		return fmt.Errorf("failed to save some master data files: %d errors encountered, first error: %w", len(allErrors), allErrors[0])
	}

	return nil
}

func (mgr *SekaiClientManager) streamNuverseMasterData(client *SekaiClient, cdnVersion int) error {
	if err := os.MkdirAll(mgr.ServerConfig.MasterDir, 0755); err != nil {
		return fmt.Errorf("failed to create master data directory: %w", err)
	}
	ctx := context.Background()
	u := fmt.Sprintf("%s/master-data-%d.info", client.ServerConfig.NuverseMasterDataURL, cdnVersion)
	cli := *client.Session
	if client.Proxy != "" {
		cli.SetProxy(client.Proxy)
	}
	req := *cli.R()
	req.SetContext(ctx)
	resp, err := req.Get(u)
	if err != nil {
		return fmt.Errorf("request error: %w", err)
	}
	if resp == nil {
		return fmt.Errorf("nil response")
	}
	status := resp.StatusCode()
	if status < 200 || status >= 300 {
		return fmt.Errorf("non-success status=%d", status)
	}
	masterOM, err := client.Cryptor.UnpackOrdered(resp.Body())
	if err != nil {
		return fmt.Errorf("unpack nuverse master info failed: %w", err)
	}
	if masterOM == nil {
		return fmt.Errorf("unexpected nuverse master info: nil ordered map")
	}
	restored, err := NuverseMasterRestorer(masterOM, client.ServerConfig.NuverseStructureFilePath)
	if err != nil {
		return fmt.Errorf("NuverseMasterRestorer error: %w", err)
	}
	masterOM = nil
	keys := restored.Keys()
	var processedFiles sync.Map
	var wg sync.WaitGroup
	var mu sync.Mutex
	var restoredMu sync.Mutex
	var allErrors []error
	var savedCount int32
	sem := make(chan struct{}, 2)
	batchSize := 30
	for i := 0; i < len(keys); i += batchSize {
		end := i + batchSize
		if end > len(keys) {
			end = len(keys)
		}

		batchKeys := keys[i:end]

		for _, key := range batchKeys {
			if _, loaded := processedFiles.LoadOrStore(key, true); loaded {
				continue
			}
			restoredMu.Lock()
			value, ok := restored.Get(key)
			if ok {
				restored.Delete(key)
			}
			restoredMu.Unlock()
			if !ok {
				continue
			}
			wg.Add(1)
			go func(k string, v any) {
				defer wg.Done()
				sem <- struct{}{}
				defer func() {
					<-sem
					v = nil
				}()

				filePath := filepath.Join(mgr.ServerConfig.MasterDir, k+".json")
				if err := mgr.saveFile(filePath, v); err != nil {
					mgr.Logger.Errorf("Failed to save %s: %v", k, err)
					mu.Lock()
					allErrors = append(allErrors, fmt.Errorf("failed to save %s: %w", k, err))
					mu.Unlock()
				} else {
					atomic.AddInt32(&savedCount, 1)
				}
			}(key, value)

			value = nil
		}
		wg.Wait()
		if i > 0 && i%30 == 0 || (i+batchSize) >= len(keys) {
			runtime.GC()
		}
	}
	restored = nil
	keys = nil
	if len(allErrors) > 0 {
		mgr.Logger.Errorf("Encountered %d errors while processing nuverse master data (saved %d/%d files)", len(allErrors), savedCount, len(keys))
		for i, err := range allErrors {
			if i < 10 {
				mgr.Logger.Errorf("Error %d: %v", i+1, err)
			}
		}
		return fmt.Errorf("failed to save some nuverse master data files: %d errors encountered, first error: %w", len(allErrors), allErrors[0])
	}

	return nil
}
