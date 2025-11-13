package client

import (
	"context"
	"errors"
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
	"github.com/go-resty/resty/v2"
)

type SekaiClientManager struct {
	Server              utils.HarukiSekaiServerRegion
	ServerConfig        utils.HarukiSekaiServerConfig
	VersionHelper       *SekaiVersionHelper
	CookieHelper        *SekaiCookieHelper
	Clients             []*SekaiClient
	AssetUpdaterServers []utils.HarukiAssetUpdaterInfo
	Git                 *git.HarukiGitUpdater
	ClientNo            int
	ClientNoLock        sync.Mutex
	Proxy               string
	Logger              *logger.Logger
}

func NewSekaiClientManager(server utils.HarukiSekaiServerRegion, serverConfig utils.HarukiSekaiServerConfig, assetUpdaterServers []utils.HarukiAssetUpdaterInfo, git *git.HarukiGitUpdater, proxy string, jpSekaiCookieURL string) *SekaiClientManager {
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

func (mgr *SekaiClientManager) parseAccountFile(path string, data []byte) []SekaiAccountInterface {
	var accounts []SekaiAccountInterface
	var raw any
	if err := sonic.Unmarshal(data, &raw); err != nil {
		mgr.Logger.Warnf("parseAccounts: json decode error %s: %v", path, err)
		return accounts
	}

	switch v := raw.(type) {
	case map[string]any:
		if acc := mgr.parseAccountMap(v, path, -1); acc != nil {
			accounts = append(accounts, acc)
		}
	case []any:
		for idx, item := range v {
			if m, ok := item.(map[string]any); ok {
				if acc := mgr.parseAccountMap(m, path, idx); acc != nil {
					accounts = append(accounts, acc)
				}
			} else {
				mgr.Logger.Warnf("parseAccounts: [%s][%d] unexpected array element type: %T", path, idx, item)
			}
		}
	default:
		mgr.Logger.Warnf("parseAccounts: unexpected top-level type in %s: %T", path, v)
	}

	return accounts
}

func (mgr *SekaiClientManager) parseAccountMap(m map[string]any, path string, idx int) SekaiAccountInterface {
	b, _ := sonic.Marshal(m)

	if mgr.Server == utils.HarukiSekaiServerRegionJP || mgr.Server == utils.HarukiSekaiServerRegionEN {
		acc := new(SekaiAccountCP)
		if unmarshalErr := sonic.Unmarshal(b, acc); unmarshalErr == nil {
			return acc
		} else {
			if idx >= 0 {
				mgr.Logger.Warnf("parseAccounts: [%s][%d] CP unmarshal error: %v", path, idx, unmarshalErr)
			} else {
				mgr.Logger.Warnf("parseAccounts: CP unmarshal error %s: %v", path, unmarshalErr)
			}
		}
	} else {
		acc := new(SekaiAccountNuverse)
		if unmarshalErr := sonic.Unmarshal(b, acc); unmarshalErr == nil {
			return acc
		} else {
			if idx >= 0 {
				mgr.Logger.Warnf("parseAccounts: [%s][%d] Nuverse unmarshal error: %v", path, idx, unmarshalErr)
			} else {
				mgr.Logger.Warnf("parseAccounts: Nuverse unmarshal error %s: %v", path, unmarshalErr)
			}
		}
	}

	return nil
}

func (mgr *SekaiClientManager) parseAccounts() ([]SekaiAccountInterface, error) {
	var accounts []SekaiAccountInterface

	err := filepath.Walk(mgr.ServerConfig.AccountDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			mgr.Logger.Warnf("parseAccounts: walk error on %s: %v", path, err)
			return err
		}
		if info.IsDir() || filepath.Ext(path) != ".json" {
			return nil
		}

		data, err := os.ReadFile(path)
		if err != nil {
			mgr.Logger.Warnf("parseAccounts: read error %s: %v", path, err)
			return nil
		}

		parsedAccounts := mgr.parseAccountFile(path, data)
		accounts = append(accounts, parsedAccounts...)
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
	mgr.Logger.Infof("Initializing client manager...")

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
	mgr.ClientNoLock.Lock()
	defer mgr.ClientNoLock.Unlock()

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

func (mgr *SekaiClientManager) handleUpgradeError() (HarukiSekaiAPIFailedResponse, int, error) {
	mgr.Logger.Warnf("%s Server upgrade required, re-parsing version...", strings.ToUpper(string(mgr.Server)))
	if err := mgr.parseVersion(); err != nil {
		resp := HarukiSekaiAPIFailedResponse{
			Result:  "failed",
			Status:  http.StatusServiceUnavailable,
			Message: fmt.Sprintf("Failed to parse version after upgrade: %v", err),
		}
		return resp, http.StatusServiceUnavailable, err
	}
	return HarukiSekaiAPIFailedResponse{}, 0, nil
}

func (mgr *SekaiClientManager) handleSessionError(ctx context.Context) (HarukiSekaiAPIFailedResponse, int, error) {
	mgr.Logger.Warnf("%s Server cookies expired, re-parsing...", strings.ToUpper(string(mgr.Server)))
	if err := mgr.parseCookies(ctx); err != nil {
		resp := HarukiSekaiAPIFailedResponse{
			Result:  "failed",
			Status:  http.StatusForbidden,
			Message: fmt.Sprintf("Failed to parse cookies: %v", err),
		}
		return resp, http.StatusForbidden, err
	}
	return HarukiSekaiAPIFailedResponse{}, 0, nil
}

func (mgr *SekaiClientManager) processSuccessResponse(client *SekaiClient, response *resty.Response, statusCode int) (any, int, error) {
	result, err := client.handleResponse(*response)
	if err != nil {
		resp := HarukiSekaiAPIFailedResponse{
			Result:  "failed",
			Status:  statusCode,
			Message: err.Error(),
		}
		return resp, statusCode, err
	}
	return result, statusCode, nil
}

func (mgr *SekaiClientManager) handleGetError(getErr error, retryCount, maxRetries int) (HarukiSekaiAPIFailedResponse, int, error, bool) {
	var ue *UpgradeRequiredError
	if errors.As(getErr, &ue) {
		if resp, status, err := mgr.handleUpgradeError(); err != nil {
			return resp, status, err, true
		}
		return HarukiSekaiAPIFailedResponse{}, 0, nil, false
	}

	if retryCount >= maxRetries-1 {
		resp := HarukiSekaiAPIFailedResponse{
			Result:  "failed",
			Status:  http.StatusInternalServerError,
			Message: fmt.Sprintf("Failed to get response: %v", getErr),
		}
		return resp, http.StatusInternalServerError, getErr, true
	}

	return HarukiSekaiAPIFailedResponse{}, 0, nil, false
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

		response, getErr := client.Get(ctx, path, params)

		if getErr != nil || response == nil {
			resp, status, err, shouldReturn := mgr.handleGetError(getErr, retryCount, maxRetries)
			if shouldReturn {
				return resp, status, err
			}

			retryCount++
			time.Sleep(retryDelay)
			continue
		}

		statusCode, err := ParseSekaiApiHttpStatus(response.StatusCode())
		if err != nil {
			resp := HarukiSekaiAPIFailedResponse{
				Result:  "failed",
				Status:  response.StatusCode(),
				Message: fmt.Sprintf("Unknown status code: %d", response.StatusCode()),
			}
			return resp, response.StatusCode(), err
		}

		switch statusCode {
		case SekaiApiHttpStatusGameUpgrade:
			if resp, status, err := mgr.handleUpgradeError(); err != nil {
				return resp, status, err
			}
			retryCount++
			time.Sleep(retryDelay)
			continue

		case SekaiApiHttpStatusSessionError:
			if resp, status, err := mgr.handleSessionError(ctx); err != nil {
				return resp, status, err
			}
			retryCount++
			time.Sleep(retryDelay)
			continue

		case SekaiApiHttpStatusUnderMaintenance:
			resp := HarukiSekaiAPIFailedResponse{
				Result:  "failed",
				Status:  http.StatusServiceUnavailable,
				Message: fmt.Sprintf("%s Game server is under maintenance.", strings.ToUpper(string(mgr.Server))),
			}
			return resp, http.StatusServiceUnavailable, NewUnderMaintenanceError()

		case SekaiApiHttpStatusOk:
			result, status, err := mgr.processSuccessResponse(client, response, response.StatusCode())
			return result, status, err

		default:
			resp := HarukiSekaiAPIFailedResponse{
				Result:  "failed",
				Status:  response.StatusCode(),
				Message: fmt.Sprintf("Game server API return status code: %d", response.StatusCode()),
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
	if client == nil {
		return nil, fmt.Errorf("no client available")
	}
	return client.GetNuverseMySekaiImage(userID, index)
}
