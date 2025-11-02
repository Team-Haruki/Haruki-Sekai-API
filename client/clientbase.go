package client

import (
	"context"
	"errors"
	"fmt"
	"haruki-sekai-api/utils"
	"haruki-sekai-api/utils/logger"
	"net"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/go-resty/resty/v2"
	"github.com/google/uuid"
	"github.com/jtacoma/uritemplates"
	"github.com/samber/lo"
)

type SekaiClient struct {
	Server        utils.HarukiSekaiServerRegion
	ServerConfig  utils.HarukiSekaiServerConfig
	Account       SekaiAccountInterface
	CookieHelper  *SekaiCookieHelper
	VersionHelper *SekaiVersionHelper
	Proxy         string
	Logger        *logger.Logger
	Cryptor       *SekaiCryptor
	APILock       *sync.Mutex
	HeaderLock    *sync.Mutex
	Session       *resty.Client
	Headers       map[string]string
}

func NewSekaiClient(
	server utils.HarukiSekaiServerRegion,
	serverConfig utils.HarukiSekaiServerConfig,
	account SekaiAccountInterface,
	cookieHelper *SekaiCookieHelper,
	versionHelper *SekaiVersionHelper,
	proxy string,
) *SekaiClient {
	cryptor, err := NewSekaiCryptorFromHex(serverConfig.AESKeyHex, serverConfig.AESIVHex)
	if err != nil {
		panic(err)
	}
	return &SekaiClient{
		Server:        server,
		ServerConfig:  serverConfig,
		Account:       account,
		CookieHelper:  cookieHelper,
		VersionHelper: versionHelper,
		Proxy:         proxy,
		Cryptor:       cryptor,
		Headers:       serverConfig.Headers,
		APILock:       &sync.Mutex{},
		HeaderLock:    &sync.Mutex{},
	}

}

func (c *SekaiClient) ParseCookies(ctx context.Context) error {
	if c.Server != utils.HarukiSekaiServerRegionJP {
		return nil
	}
	cookie, err := c.CookieHelper.GetCookies(ctx, c.Proxy)
	if err != nil {
		return err
	}
	c.Headers["Cookie"] = cookie
	return nil
}

func (c *SekaiClient) ParseVersion() error {
	if err := c.VersionHelper.GetAppVersion(); err != nil {
		return err
	}
	c.HeaderLock.Lock()
	if c.Headers == nil {
		c.Headers = map[string]string{}
	}
	c.Headers["X-App-Version"] = c.VersionHelper.AppVersion
	c.Headers["X-Data-Version"] = c.VersionHelper.DataVersion
	c.Headers["X-Asset-Version"] = c.VersionHelper.AssetVersion
	c.Headers["X-App-Hash"] = c.VersionHelper.AppHash
	c.HeaderLock.Unlock()
	return nil
}

func (c *SekaiClient) Init() error {
	c.Session = resty.New()
	c.Session.
		SetRetryCount(0).
		SetTransport(&http.Transport{
			MaxIdleConns:        100,
			MaxIdleConnsPerHost: 20,
			IdleConnTimeout:     90 * time.Second,
			TLSHandshakeTimeout: 5 * time.Second,
			DisableKeepAlives:   false,
		})

	if c.Proxy != "" {
		c.Session.SetProxy(c.Proxy)
	}
	c.Logger = logger.NewLogger(fmt.Sprintf("SekaiClient%s", strings.ToUpper(string(c.Server))), "DEBUG", nil)
	if err := c.ParseCookies(context.Background()); err != nil {
		return err
	}
	if err := c.ParseVersion(); err != nil {
		return err
	}
	return nil
}

func (c *SekaiClient) handleResponse(response resty.Response) (any, error) {
	statusCode, err := ParseSekaiApiHttpStatus(response.StatusCode())
	if err != nil {
		c.Logger.Errorf("Parse status code error : %v", err)
		return nil, err
	}
	contentType := strings.ToLower(response.Header().Get("Content-Type"))

	if lo.Contains([]string{"application/octet-stream", "binary/octet-stream"}, contentType) {
		unpackResponse, err := c.Cryptor.Unpack(response.Body())
		if err != nil {
			c.Logger.Errorf("Unpack response error : %v", err)
			return nil, err
		}
		switch statusCode {
		case SekaiApiHttpStatusOk,
			SekaiApiHttpStatusClientError,
			SekaiApiHttpStatusNotFound,
			SekaiApiHttpStatusConflict:
			return &unpackResponse, nil
		case SekaiApiHttpStatusSessionError:
			return nil, NewSessionError()
		case SekaiApiHttpStatusGameUpgrade:
			return nil, NewUpgradeRequiredError()
		case SekaiApiHttpStatusUnderMaintenance:
			return nil, NewUnderMaintenanceError()
		default:
			return nil, NewSekaiUnknownClientException(response.StatusCode(), string(response.Body()))
		}
	} else {
		if statusCode == SekaiApiHttpStatusServerError {
			return nil, NewSekaiUnknownClientException(response.StatusCode(), string(response.Body()))
		}
		if statusCode == SekaiApiHttpStatusSessionError && contentType == "text/xml" {
			return nil, NewCookieExpiredError()
		}
	}
	return nil, NewSekaiUnknownClientException(response.StatusCode(), string(response.Body()))
}

func (c *SekaiClient) prepareRequest(ctx context.Context, data any, params map[string]any) (*resty.Request, error) {
	req := c.Session.R()
	req.SetContext(ctx)
	req.SetHeaders(c.Headers)
	req.Header.Set("X-Request-Id", uuid.New().String())

	if params != nil {
		for k, v := range params {
			req.SetQueryParam(k, fmt.Sprintf("%v", v))
		}
	}

	if data != nil {
		packedData, err := c.Cryptor.Pack(data)
		if err != nil {
			c.Logger.Errorf("pack error: %v", err)
			return nil, err
		}
		req.SetBody(packedData)
	}

	return req, nil
}

func (c *SekaiClient) handleExecutionError(execErr error, attempt int) error {
	var ne net.Error
	if errors.Is(execErr, context.DeadlineExceeded) || (errors.As(execErr, &ne) && ne.Timeout()) {
		c.Logger.Warnf("account #%s request timed out (attempt %d), retrying...", c.Account.GetUserId(), attempt)
		return execErr
	}
	c.Logger.Errorf("request error (attempt %d): server=%s, err=%v", attempt, strings.ToUpper(string(c.Server)), execErr)
	return execErr
}

func (c *SekaiClient) updateSessionToken(response *resty.Response) {
	if v := response.Header().Get("X-Session-Token"); v != "" {
		c.HeaderLock.Lock()
		c.Headers["X-Session-Token"] = v
		c.HeaderLock.Unlock()
	}
}

func (c *SekaiClient) handleSessionError() error {
	c.Logger.Warnf("account #%s session expired, re-logging in...", c.Account.GetUserId())
	if _, err := c.Login(context.Background()); err != nil {
		c.Logger.Errorf("re-login failed: %v", err)
		return err
	}
	return NewSessionError()
}

func (c *SekaiClient) handleCookieExpiredError(ctx context.Context) error {
	c.Logger.Warnf("cookies expired, re-parsing cookies...")
	if err := c.ParseCookies(ctx); err != nil {
		c.Logger.Errorf("parse cookies failed: %v", err)
		return err
	}
	return NewCookieExpiredError()
}

func (c *SekaiClient) handleUpgradeError(ctx context.Context) error {
	if c.Server == utils.HarukiSekaiServerRegionJP || c.Server == utils.HarukiSekaiServerRegionEN {
		c.Logger.Warnf("app version might be upgraded")
		return NewUpgradeRequiredError()
	}
	c.Logger.Warnf("%s server detected new data, re-logging in...", strings.ToUpper(string(c.Server)))
	if _, err := c.Login(ctx); err != nil {
		c.Logger.Errorf("re-login failed: %v", err)
		return err
	}
	return NewSessionError()
}

func (c *SekaiClient) handleResponseError(ctx context.Context, respErr error, response *resty.Response, attempt int) (error, bool) {
	var (
		se *SessionError
		ce *CookieExpiredError
		ue *UpgradeRequiredError
		me *UnderMaintenanceError
	)

	switch {
	case errors.As(respErr, &se):
		return c.handleSessionError(), false
	case errors.As(respErr, &ce):
		err := c.handleCookieExpiredError(ctx)
		// Return immediately if cookie parsing failed
		if err != nil && err.Error() != "cookie expired" {
			return err, true
		}
		return err, false
	case errors.As(respErr, &ue):
		err := c.handleUpgradeError(ctx)
		// Return immediately for JP/EN servers
		if c.Server == utils.HarukiSekaiServerRegionJP || c.Server == utils.HarukiSekaiServerRegionEN {
			return err, true
		}
		return err, false
	case errors.As(respErr, &me):
		c.Logger.Warnf("server is under maintenance")
		return me, true
	default:
		if sc := response.StatusCode(); sc >= 500 {
			c.Logger.Warnf("server error %d on attempt %d", sc, attempt)
			return NewSekaiUnknownClientException(sc, string(response.Body())), false
		}
		return respErr, true
	}
}

func (c *SekaiClient) CallAPI(ctx context.Context, path string, method string, data any, params map[string]any) (*resty.Response, error) {
	uri := fmt.Sprintf("%s/api%s", c.ServerConfig.APIURL, path)

	template, err := uritemplates.Parse(uri)
	if err != nil {
		return nil, err
	}
	values := map[string]any{
		"userId": c.Account.GetUserId(),
	}

	url, err := template.Expand(values)
	if err != nil {
		return nil, err
	}
	c.Logger.Infof("account #%s %s %s", c.Account.GetUserId(), strings.ToUpper(method), path)

	if c.Session == nil {
		return nil, fmt.Errorf("resty client is nil")
	}

	var lastErr error
	for attempt := 1; attempt <= 4; attempt++ {
		req, err := c.prepareRequest(ctx, data, params)
		if err != nil {
			return nil, err
		}

		response, execErr := req.Execute(strings.ToUpper(method), url)
		if execErr != nil {
			lastErr = c.handleExecutionError(execErr, attempt)
		} else {
			c.updateSessionToken(response)

			if _, respErr := c.handleResponse(*response); respErr != nil {
				err, shouldReturn := c.handleResponseError(ctx, respErr, response, attempt)
				if shouldReturn {
					return nil, err
				}
				lastErr = err
			} else {
				return response, nil
			}
		}

		if attempt < 4 {
			time.Sleep(time.Second)
		}
	}

	if lastErr != nil {
		return nil, lastErr
	}
	return nil, fmt.Errorf("request failed after retries")
}

func (c *SekaiClient) Get(ctx context.Context, path string, params map[string]any) (*resty.Response, error) {
	return c.CallAPI(ctx, path, "GET", nil, params)
}

func (c *SekaiClient) Post(ctx context.Context, path string, data any, params map[string]any) (*resty.Response, error) {
	return c.CallAPI(ctx, path, "POST", data, params)
}

func (c *SekaiClient) Put(ctx context.Context, path string, data any, params map[string]any) (*resty.Response, error) {
	return c.CallAPI(ctx, path, "PUT", data, params)
}

func (c *SekaiClient) Delete(ctx context.Context, path string, params map[string]any) (*resty.Response, error) {
	return c.CallAPI(ctx, path, "DELETE", nil, params)
}

func (c *SekaiClient) Patch(ctx context.Context, path string, data any, params map[string]any) (*resty.Response, error) {
	return c.CallAPI(ctx, path, "PATCH", data, params)
}

func (c *SekaiClient) Close() error {
	if c.Session != nil {
		c.Session = nil
	}
	return nil
}
