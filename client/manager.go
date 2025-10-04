package client

import (
	"context"
	"encoding/json"
	"fmt"
	"haruki-sekai-api/logger"
	"haruki-sekai-api/utils"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

type SekaiClientManager struct {
	Server        utils.SekaiRegion
	ServerInfo    SekaiServerInfo
	VersionHelper *SekaiVersionHelper
	CookieHelper  *SekaiCookieHelper
	AccountsDir   string
	Clients       []*SekaiClient
	ClientNo      int
	Proxies       []string
	Logger        *logger.Logger
}

func NewSekaiClientManager(serverInfo SekaiServerInfo, accountsDir, versionFilePath string, proxies []string) *SekaiClientManager {
	mgr := &SekaiClientManager{
		Server:        serverInfo.Server,
		ServerInfo:    serverInfo,
		VersionHelper: &SekaiVersionHelper{versionFilePath: versionFilePath},
		AccountsDir:   accountsDir,
		Proxies:       proxies,
		Logger:        logger.NewLogger("SekaiClientManager", "DEBUG", nil),
	}
	if serverInfo.Server == "jp" {
		mgr.CookieHelper = &SekaiCookieHelper{}
	}
	return mgr
}

func (mgr *SekaiClientManager) parseAccounts() ([]SekaiAccountInterface, error) {
	var accounts []SekaiAccountInterface
	err := filepath.Walk(mgr.AccountsDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if info.IsDir() || filepath.Ext(path) != ".json" {
			return nil
		}

		data, err := os.ReadFile(path)
		if err != nil {
			mgr.Logger.Warnf("Error reading file %s: %v", path, err)
			return nil
		}

		var raw any
		if err := json.Unmarshal(data, &raw); err != nil {
			mgr.Logger.Warnf("Error decoding JSON in file %s: %v", path, err)
			return nil
		}

		switch v := raw.(type) {
		case map[string]any:
			if mgr.Server == utils.SekaiRegionJP || mgr.Server == utils.SekaiRegionEN {
				var acc *SekaiAccountCP
				b, _ := json.Marshal(v)
				if err := json.Unmarshal(b, acc); err == nil {
					accounts = append(accounts, acc)
				}
			} else {
				var acc *SekaiAccountNuverse
				b, _ := json.Marshal(v)
				if err := json.Unmarshal(b, acc); err == nil {
					accounts = append(accounts, acc)
				}
			}
		case []any:
			for _, item := range v {
				if m, ok := item.(map[string]any); ok {
					if mgr.Server == utils.SekaiRegionJP || mgr.Server == utils.SekaiRegionEN {
						var acc *SekaiAccountCP
						b, _ := json.Marshal(m)
						if err := json.Unmarshal(b, acc); err == nil {
							accounts = append(accounts, acc)
						}
					} else {
						var acc *SekaiAccountNuverse
						b, _ := json.Marshal(m)
						if err := json.Unmarshal(b, acc); err == nil {
							accounts = append(accounts, acc)
						}
					}
				}
			}
		default:
			mgr.Logger.Warnf("Unexpected data type in file %s: %T", path, v)
		}
		return nil
	})
	return accounts, err
}

func (mgr *SekaiClientManager) ParseCookies(ctx context.Context) error {
	if mgr.Server == utils.SekaiRegionJP {
		var wg sync.WaitGroup
		errChan := make(chan error, len(mgr.Clients))
		for _, client := range mgr.Clients {
			wg.Add(1)
			go func(c *SekaiClient) {
				defer wg.Done()
				if err := c.ParseCookies(ctx); err != nil {
					mgr.Logger.Warnf("Error parsing cookies: %v", err)
					errChan <- err
				}
			}(client)
		}
		wg.Wait()
		close(errChan)

		for err := range errChan {
			if err != nil {
				return err
			}
		}
	}
	return nil
}

func (mgr *SekaiClientManager) ParseVersion() error {
	var wg sync.WaitGroup
	errChan := make(chan error, len(mgr.Clients))
	for _, client := range mgr.Clients {
		wg.Add(1)
		go func(c *SekaiClient) {
			defer wg.Done()
			if err := c.ParseVersion(); err != nil {
				mgr.Logger.Warnf("Error parsing version: %v", err)
				errChan <- err
			}
		}(client)
	}
	wg.Wait()
	close(errChan)

	for err := range errChan {
		if err != nil {
			return err
		}
	}
	return nil
}

func (mgr *SekaiClientManager) Init() error {
	mgr.Logger.Debugf("Initializing client manager...")

	accounts, err := mgr.parseAccounts()
	if err != nil {
		return err
	}

	for _, account := range accounts {
		client := NewSekaiClient(
			&mgr.ServerInfo,
			account,
			mgr.CookieHelper,
			mgr.VersionHelper,
			&mgr.Proxies,
		)
		mgr.Clients = append(mgr.Clients, client)
	}

	var wg sync.WaitGroup
	initErrors := make(chan error, len(mgr.Clients))
	for _, client := range mgr.Clients {
		wg.Add(1)
		go func(c *SekaiClient) {
			defer wg.Done()
			if err := c.Init(); err != nil {
				mgr.Logger.Errorf("Error initializing client: %v", err)
				initErrors <- err
			}
		}(client)
	}
	wg.Wait()
	close(initErrors)

	for err := range initErrors {
		if err != nil {
			return err
		}
	}

	ctx := context.Background()
	loginErrors := make(chan error, len(mgr.Clients))
	for _, client := range mgr.Clients {
		wg.Add(1)
		go func(c *SekaiClient) {
			defer wg.Done()
			if _, err := c.Login(ctx); err != nil {
				mgr.Logger.Errorf("Error logging in: %v", err)
				loginErrors <- err
			}
		}(client)
	}
	wg.Wait()
	close(loginErrors)

	for err := range loginErrors {
		if err != nil {
			return err
		}
	}

	mgr.Logger.Infof("[INFO] Client manager initialized successfully")
	return nil
}

func (mgr *SekaiClientManager) GetClient() *SekaiClient {
	if mgr.ClientNo == len(mgr.Clients) {
		mgr.ClientNo = 0
		return mgr.Clients[mgr.ClientNo]
	}
	mgr.ClientNo++
	return mgr.Clients[mgr.ClientNo-1]
}

func (mgr *SekaiClientManager) GetLoginData() (map[string]interface{}, error) {
	client := mgr.GetClient()
	if client == nil {
		return nil, nil
	}

	client.Lock.Lock()
	defer client.Lock.Unlock()

	ctx := context.Background()
	loginData, err := client.Login(ctx)
	if err != nil {
		return nil, err
	}
	if m, ok := loginData.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

func (mgr *SekaiClientManager) DownloadMaster() (map[string]interface{}, error) {
	client := mgr.GetClient()
	if client == nil {
		return nil, nil
	}

	client.Lock.Lock()
	defer client.Lock.Unlock()

	ctx := context.Background()
	masterData, err := client.GetMasterData(ctx)
	if err != nil {
		return nil, err
	}
	if m, ok := masterData.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

func (mgr *SekaiClientManager) Shutdown() error {
	var wg sync.WaitGroup
	errChan := make(chan error, len(mgr.Clients))

	for _, client := range mgr.Clients {
		wg.Add(1)
		go func(c *SekaiClient) {
			defer wg.Done()
			if err := c.Close(); err != nil {
				mgr.Logger.Warnf("Error closing client: %v", err)
				errChan <- err
			}
		}(client)
	}
	wg.Wait()
	close(errChan)

	for err := range errChan {
		if err != nil {
			return err
		}
	}

	mgr.Logger.Debugf("Client manager shut down successfully")
	return nil
}

// APIGet 调用游戏API并处理重试逻辑
func (mgr *SekaiClientManager) APIGet(ctx context.Context, path string, params map[string]interface{}) (any, int, error) {
	maxRetries := 4
	retryCount := 0
	retryDelay := time.Second

	for retryCount < maxRetries {
		client := mgr.GetClient()
		if client == nil {
			return map[string]interface{}{
				"result":  "failed",
				"message": "No client is available, please try again later.",
			}, 500, nil
		}

		client.Lock.Lock()

		response, err := client.Get(ctx, path, params)
		if err != nil {
			client.Lock.Unlock()
			return map[string]interface{}{
				"result":  "failed",
				"message": err.Error(),
			}, 500, err
		}

		// 解析响应
		statusCode, err := ParseSekaiApiHttpStatus(response.StatusCode())
		if err != nil {
			client.Lock.Unlock()
			return map[string]interface{}{
				"result":  "failed",
				"message": fmt.Sprintf("Unknown status code: %d", response.StatusCode()),
			}, response.StatusCode(), err
		}

		switch statusCode {
		case SekaiApiHttpStatusGameUpgrade:
			mgr.Logger.Warnf("%s Server upgrade required, re-parsing...", strings.ToUpper(string(mgr.Server)))
			if err := mgr.ParseVersion(); err != nil {
				client.Lock.Unlock()
				return map[string]interface{}{
					"result":  "failed",
					"message": fmt.Sprintf("Failed to parse version: %v", err),
				}, response.StatusCode(), err
			}
			retryCount++
			time.Sleep(retryDelay)
			client.Lock.Unlock()
			continue

		case SekaiApiHttpStatusSessionError:
			mgr.Logger.Warnf("%s Server cookies expired, re-parsing...", strings.ToUpper(string(mgr.Server)))
			if err := mgr.ParseCookies(ctx); err != nil {
				client.Lock.Unlock()
				return map[string]interface{}{
					"result":  "failed",
					"message": fmt.Sprintf("Failed to parse cookies: %v", err),
				}, response.StatusCode(), err
			}
			retryCount++
			time.Sleep(retryDelay)
			client.Lock.Unlock()
			continue

		case SekaiApiHttpStatusUnderMaintenance:
			client.Lock.Unlock()
			return map[string]interface{}{
				"result":  "failed",
				"message": fmt.Sprintf("%s Game server is under maintenance.", strings.ToUpper(string(mgr.Server))),
			}, 503, NewUnderMaintenanceError()

		case SekaiApiHttpStatusOk:
			result, err := client.response(*response)
			client.Lock.Unlock()
			if err != nil {
				return map[string]interface{}{
					"result":  "failed",
					"message": err.Error(),
				}, response.StatusCode(), err
			}
			if m, ok := result.(map[string]interface{}); ok {
				return m, response.StatusCode(), nil
			}
			return result, response.StatusCode(), nil

		default:
			client.Lock.Unlock()
			return map[string]interface{}{
				"result":  "failed",
				"message": fmt.Sprintf("Unexpected status code: %d", response.StatusCode()),
			}, response.StatusCode(), fmt.Errorf("unexpected status code: %d", response.StatusCode())
		}
	}

	return map[string]interface{}{
		"result":  "failed",
		"message": "Max retry attempts reached",
	}, 500, fmt.Errorf("max retry attempts reached")
}
