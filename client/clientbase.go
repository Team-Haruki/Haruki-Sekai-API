package client

import (
	"context"
	"errors"
	"fmt"
	"haruki-sekai-api/utils"
	"haruki-sekai-api/utils/logger"
	"net"
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
	Lock          *sync.Mutex
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
		Lock:          &sync.Mutex{},
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
	if c.Headers == nil {
		c.Headers = map[string]string{}
	}
	c.Headers["X-App-Version"] = c.VersionHelper.AppVersion
	c.Headers["X-Data-Version"] = c.VersionHelper.DataVersion
	c.Headers["X-Asset-Version"] = c.VersionHelper.AssetVersion
	c.Headers["X-App-Hash"] = c.VersionHelper.AppHash
	return nil
}

func (c *SekaiClient) Init() error {
	c.Session = resty.New()
	c.Session.
		SetRetryCount(4).
		SetRetryWaitTime(time.Second * 1)

	for k, v := range c.ServerConfig.Headers {
		c.Session.SetHeader(k, v)
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

	c.Logger.Infof("account #%d %s %s",
		c.Account.GetUserId(),
		method,
		url,
	)

	cli := *c.Session
	if c.Proxy != "" {
		cli.SetProxy(c.Proxy)
	}

	var lastErr error
	for attempt := 1; attempt <= 4; attempt++ {
		req := *cli.R()
		req.SetContext(ctx)
		req.SetHeaders(c.Headers)
		req.Header.Set("X-Request-ID", uuid.New().String())

		for k, v := range params {
			req.SetQueryParam(k, fmt.Sprintf("%v", v))
		}
		if data != nil {
			c.Logger.Debugf("payload: %v", data)
			packedData, err := c.Cryptor.Pack(data)
			c.Logger.Debugf("payload: %v", packedData)
			if err != nil {
				return nil, err
			}
			req.SetBody(packedData)
		}
		c.Logger.Debugf("headers: %+v", req.Header)
		response, err := req.Execute(strings.ToUpper(method), url)
		unpacked, _ := c.Cryptor.Unpack(response.Body())
		c.Logger.Debugf("response raw: %v", string(response.Body()))
		c.Logger.Debugf("response decrypted: %v", unpacked)
		if err != nil {
			var ne net.Error
			if errors.Is(err, context.DeadlineExceeded) || (errors.As(err, &ne) && ne.Timeout()) {
				c.Logger.Warnf("%s client #%s request timed out, retrying...", strings.ToUpper(string(c.Server)), c.Account.GetUserId())
				lastErr = err
			} else {
				c.Logger.Errorf("An error occurred: server = %s, exception = %v", strings.ToUpper(string(c.Server)), err)
				lastErr = err
			}
		} else {
			if v := response.Header().Get("X-Session-Token"); v != "" {
				c.Headers["X-Session-Token"] = v
			}
			if _, respErr := c.handleResponse(*response); respErr != nil {
				var (
					se *SessionError
					ce *CookieExpiredError
					ue *UpgradeRequiredError
					me *UnderMaintenanceError
				)
				switch {
				case errors.As(respErr, &se):
					c.Logger.Warnf("Account #%s session expired, re-logging in...", c.Account.GetUserId())
					ctxNoRelog := context.Background()
					if _, err := c.Login(ctxNoRelog); err != nil {
						c.Logger.Errorf("Re-login failed: %v", err)
						lastErr = err
						break
					}
					lastErr = se
				case errors.As(respErr, &ce):
					c.Logger.Warnf("Clients' cookies expired, re-parsing cookies...")
					if err := c.ParseCookies(ctx); err != nil {
						c.Logger.Errorf("Parse cookies failed: %v", err)
						return nil, err
					}
					lastErr = ce
				case errors.As(respErr, &ue):
					if c.Server == utils.HarukiSekaiServerRegionJP || c.Server == utils.HarukiSekaiServerRegionEN {
						c.Logger.Warnf("App version might be upgraded")
						return nil, ue
					} else {
						c.Logger.Warnf("%s server detected new data, re-logging in...", strings.ToUpper(string(c.Server)))
						if _, err := c.Login(ctx); err != nil {
							c.Logger.Errorf("Re-login failed: %v", err)
							lastErr = err
							break
						}
						lastErr = NewSessionError()
					}
				case errors.As(respErr, &me):
					c.Logger.Warnf("Server is under maintenance")
					return nil, me
				default:
					if sc := response.StatusCode(); sc >= 500 {
						c.Logger.Warnf("Server error %d on attempt %d", sc, attempt)
						lastErr = NewSekaiUnknownClientException(sc, string(response.Body()))
					} else {
						return nil, respErr
					}
				}
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
