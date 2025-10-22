package client

import (
	"context"
	"fmt"
	"haruki-sekai-api/utils"
	"haruki-sekai-api/utils/git"
	"haruki-sekai-api/utils/logger"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/bytedance/sonic"
)

type SekaiClientManager struct {
	Server              utils.HarukiSekaiServerRegion
	ServerConfig        utils.HarukiSekaiServerConfig
	VersionHelper       *SekaiVersionHelper
	CookieHelper        *SekaiCookieHelper
	Clients             []*SekaiClient
	AssetUpdaterServers []*utils.HarukiAssetUpdaterInfo
	Git                 *git.HarukiGitUpdater
	ClientNo            int
	Proxy               string
	Logger              *logger.Logger
}

func NewSekaiClientManager(server utils.HarukiSekaiServerRegion, serverConfig utils.HarukiSekaiServerConfig, assetUpdaterServers []*utils.HarukiAssetUpdaterInfo, git *git.HarukiGitUpdater, proxy string, jpSekaiCookieURL string) *SekaiClientManager {
	mgr := &SekaiClientManager{
		Server:              server,
		ServerConfig:        serverConfig,
		VersionHelper:       &SekaiVersionHelper{versionFilePath: serverConfig.VersionPath},
		Proxy:               proxy,
		AssetUpdaterServers: assetUpdaterServers,
		Git:                 git,
		Logger:              logger.NewLogger(fmt.Sprintf("SekaiClientManager%s", strings.ToUpper(string(server))), "DEBUG", nil),
	}
	if server == utils.HarukiSekaiServerRegionJP {
		mgr.CookieHelper = &SekaiCookieHelper{url: jpSekaiCookieURL}
	}
	return mgr
}

func (mgr *SekaiClientManager) parseAccounts() ([]SekaiAccountInterface, error) {
	var (
		accounts []SekaiAccountInterface
	)

	err := filepath.Walk(mgr.ServerConfig.AccountDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			mgr.Logger.Warnf("parseAccounts: walk error on %s: %v", path, err)
			return err
		}
		if info.IsDir() {
			return nil
		}
		if filepath.Ext(path) != ".json" {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil {
			mgr.Logger.Warnf("parseAccounts: read error %s: %v", path, err)
			return nil
		}
		var raw any
		if err := sonic.Unmarshal(data, &raw); err != nil {
			mgr.Logger.Warnf("parseAccounts: json decode error %s: %v", path, err)
			return nil
		}

		switch v := raw.(type) {
		case map[string]any:
			if mgr.Server == utils.HarukiSekaiServerRegionJP || mgr.Server == utils.HarukiSekaiServerRegionEN {
				acc := new(SekaiAccountCP)
				b, _ := sonic.Marshal(v)
				if err := sonic.Unmarshal(b, acc); err == nil {
					accounts = append(accounts, acc)
				} else {
					mgr.Logger.Warnf("parseAccounts: CP unmarshal error %s: %v", path, err)
				}
			} else {
				acc := new(SekaiAccountNuverse)
				b, _ := sonic.Marshal(v)
				if err := sonic.Unmarshal(b, acc); err == nil {
					accounts = append(accounts, acc)
				} else {
					mgr.Logger.Warnf("parseAccounts: Nuverse unmarshal error %s: %v", path, err)
				}
			}
		case []any:
			for idx, item := range v {
				if m, ok := item.(map[string]any); ok {
					if mgr.Server == utils.HarukiSekaiServerRegionJP || mgr.Server == utils.HarukiSekaiServerRegionEN {
						acc := new(SekaiAccountCP)
						b, _ := sonic.Marshal(m)
						if err := sonic.Unmarshal(b, acc); err == nil {
							accounts = append(accounts, acc)
						} else {
							mgr.Logger.Warnf("parseAccounts: [%s][%d] CP unmarshal error: %v", path, idx, err)
						}
					} else {
						acc := new(SekaiAccountNuverse)
						b, _ := sonic.Marshal(m)
						if err := sonic.Unmarshal(b, acc); err == nil {
							accounts = append(accounts, acc)
						} else {
							mgr.Logger.Warnf("parseAccounts: [%s][%d] Nuverse unmarshal error: %v", path, idx, err)
						}
					}
				} else {
					mgr.Logger.Warnf("parseAccounts: [%s][%d] unexpected array element type: %T", path, idx, item)
				}
			}
		default:
			mgr.Logger.Warnf("parseAccounts: unexpected top-level type in %s: %T", path, v)
		}
		return nil
	})

	if len(accounts) == 0 {
		mgr.Logger.Warnf("parseAccounts: no accounts parsed from %s", mgr.ServerConfig.AccountDir)
	}

	return accounts, err
}

func (mgr *SekaiClientManager) parseCookies(ctx context.Context) error {
	if mgr.Server == utils.HarukiSekaiServerRegionJP {
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

func (mgr *SekaiClientManager) parseVersion() error {
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
			mgr.Server,
			mgr.ServerConfig,
			account,
			mgr.CookieHelper,
			mgr.VersionHelper,
			mgr.Proxy,
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

	mgr.Logger.Infof("Client manager initialized successfully")
	return nil
}

func (mgr *SekaiClientManager) getClient() *SekaiClient {
	if len(mgr.Clients) == 0 {
		return nil
	}
	if mgr.ClientNo >= len(mgr.Clients) || mgr.ClientNo < 0 {
		mgr.ClientNo = 0
	}
	idx := mgr.ClientNo % len(mgr.Clients)
	c := mgr.Clients[idx]
	mgr.ClientNo = (idx + 1) % len(mgr.Clients)
	return c
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

func (mgr *SekaiClientManager) GetGameAPI(ctx context.Context, path string, params map[string]any) (any, int, error) {
	if len(mgr.Clients) == 0 {
		resp := HarukiSekaiAPIFailedResponse{
			Result:  "failed",
			Status:  http.StatusInternalServerError,
			Message: "No client initialized",
		}
		return resp, http.StatusInternalServerError, nil
	}

	maxRetries := 4
	retryCount := 0
	retryDelay := time.Second

	for retryCount < maxRetries {
		client := mgr.getClient()
		if client == nil {
			resp := HarukiSekaiAPIFailedResponse{
				Result:  "failed",
				Status:  http.StatusInternalServerError,
				Message: "No client is available, please try again later.",
			}
			return resp, http.StatusInternalServerError, nil
		}

		client.Lock.Lock()

		response, err := client.Get(ctx, path, params)
		if err != nil {
			client.Lock.Unlock()
			resp := HarukiSekaiAPIFailedResponse{
				Result:  "failed",
				Status:  http.StatusInternalServerError,
				Message: err.Error(),
			}
			return resp, http.StatusInternalServerError, err
		}

		statusCode, err := ParseSekaiApiHttpStatus(response.StatusCode())
		if err != nil {
			client.Lock.Unlock()
			resp := HarukiSekaiAPIFailedResponse{
				Result:  "failed",
				Status:  response.StatusCode(),
				Message: fmt.Sprintf("Unknown status code: %d", response.StatusCode()),
			}
			return resp, response.StatusCode(), err
		}

		switch statusCode {
		case SekaiApiHttpStatusGameUpgrade:
			mgr.Logger.Warnf("%s Server upgrade required, re-parsing...", strings.ToUpper(string(mgr.Server)))
			if err := mgr.parseVersion(); err != nil {
				client.Lock.Unlock()
				resp := HarukiSekaiAPIFailedResponse{
					Result:  "failed",
					Status:  response.StatusCode(),
					Message: fmt.Sprintf("Failed to parse version: %v", err),
				}
				return resp, response.StatusCode(), err
			}
			retryCount++
			time.Sleep(retryDelay)
			client.Lock.Unlock()
			continue

		case SekaiApiHttpStatusSessionError:
			mgr.Logger.Warnf("%s Server cookies expired, re-parsing...", strings.ToUpper(string(mgr.Server)))
			if err := mgr.parseCookies(ctx); err != nil {
				client.Lock.Unlock()
				resp := HarukiSekaiAPIFailedResponse{
					Result:  "failed",
					Status:  http.StatusForbidden,
					Message: fmt.Sprintf("Failed to parse cookies: %v", err),
				}
				return resp, http.StatusForbidden, err
			}
			retryCount++
			time.Sleep(retryDelay)
			client.Lock.Unlock()
			continue

		case SekaiApiHttpStatusUnderMaintenance:
			client.Lock.Unlock()
			resp := HarukiSekaiAPIFailedResponse{
				Result:  "failed",
				Status:  http.StatusServiceUnavailable,
				Message: fmt.Sprintf("%s Game server is under maintenance.", strings.ToUpper(string(mgr.Server))),
			}
			return resp, http.StatusServiceUnavailable, NewUnderMaintenanceError()

		case SekaiApiHttpStatusOk:
			result, err := client.handleResponse(*response)
			client.Lock.Unlock()
			if err != nil {
				resp := HarukiSekaiAPIFailedResponse{
					Result:  "failed",
					Status:  response.StatusCode(),
					Message: err.Error(),
				}
				return resp, response.StatusCode(), err
			}
			if m, ok := result.(map[string]any); ok {
				return m, response.StatusCode(), nil
			}
			return result, response.StatusCode(), nil

		default:
			client.Lock.Unlock()
			resp := HarukiSekaiAPIFailedResponse{
				Result:  "failed",
				Status:  response.StatusCode(),
				Message: fmt.Sprintf("Unexpected status code: %d", response.StatusCode()),
			}
			return resp, response.StatusCode(), fmt.Errorf("unexpected status code: %d", response.StatusCode())
		}
	}

	resp := HarukiSekaiAPIFailedResponse{
		Result:  "failed",
		Status:  http.StatusInternalServerError,
		Message: "Max retry attempts reached",
	}
	return resp, http.StatusInternalServerError, fmt.Errorf("max retry attempts reached")
}

func (mgr *SekaiClientManager) GetCPMySekaiImage(path string) ([]byte, error) {
	client := mgr.getClient()
	return client.GetCPMySekaiImage(path)
}

func (mgr *SekaiClientManager) GetNuverseMySekaiImage(userID, index string) ([]byte, error) {
	client := mgr.getClient()
	return client.GetNuverseMySekaiImage(userID, index)
}
