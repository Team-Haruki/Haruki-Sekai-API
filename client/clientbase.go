package client

import (
	"context"
	"errors"
	"fmt"
	"haruki-sekai-api/logger"
	"haruki-sekai-api/utils"
	"net"
	netUrl "net/url"
	"strings"
	"sync"
	"time"

	"github.com/go-resty/resty/v2"
	"github.com/google/uuid"
	"github.com/jtacoma/uritemplates"
	"github.com/samber/lo"
)

type SekaiClient struct {
	ServerInfo    *SekaiServerInfo
	Account       SekaiAccountInterface
	CookieHelper  *SekaiCookieHelper
	VersionHelper *SekaiVersionHelper
	Proxy         string
	Logger        *logger.Logger

	Lock    *sync.Mutex
	Session *resty.Client
}

func NewSekaiClient(
	serverInfo *SekaiServerInfo,
	account SekaiAccountInterface,
	cookieHelper *SekaiCookieHelper,
	versionHelper *SekaiVersionHelper,
	proxy string,
) *SekaiClient {
	return &SekaiClient{
		ServerInfo:    serverInfo,
		Account:       account,
		CookieHelper:  cookieHelper,
		VersionHelper: versionHelper,
		Proxy:         proxy,
		Lock:          &sync.Mutex{},
	}

}

func (c *SekaiClient) ParseCookies(ctx context.Context) error {
	if c.ServerInfo.Server != utils.SekaiRegionJP {
		return nil
	}
	cookie, err := c.CookieHelper.GetCookies(ctx, c.Proxy)
	if err != nil {
		return err
	}

	c.Session.SetHeader("Cookie", cookie)
	return nil
}

func (c *SekaiClient) ParseVersion() error {
	if err := c.VersionHelper.GetAppVersion(); err != nil {
		return err
	}
	c.Session.SetHeaders(map[string]string{
		"X-App-Version":   c.VersionHelper.AppVersion,
		"X-Data-Version":  c.VersionHelper.DataVersion,
		"X-Asset-Version": c.VersionHelper.AssetVersion,
		"X-App-Hash":      c.VersionHelper.AppHash,
	})
	return nil
}

func (c *SekaiClient) Init() error {
	c.Session = resty.New()
	c.Session.
		SetRetryCount(4).
		SetRetryWaitTime(time.Second * 1)

	for k, v := range c.ServerInfo.Headers {
		c.Session.SetHeader(k, v)
	}

	c.Logger = logger.NewLogger("SekaiClient", "DEBUG", nil)
	if err := c.ParseCookies(context.Background()); err != nil {
		return err
	}
	if err := c.ParseVersion(); err != nil {
		return err
	}
	return nil
}

func (c *SekaiClient) response(response resty.Response) (any, error) {
	statusCode, err := ParseSekaiApiHttpStatus(response.StatusCode())
	if err != nil {
		c.Logger.Errorf("Parse status code error : %v", err)
		return nil, err
	}

	if lo.Contains([]string{"application/octet-stream", "binary/octet-stream"}, c.Session.Header.Get("Content-Type")) {
		unpackResponse, err := Unpack(response.Body(), c.ServerInfo.Server)
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
		if statusCode == SekaiApiHttpStatusSessionError && c.Session.Header.Get("Content-Type") == "text/xml" {
			return nil, NewCookieExpiredError()
		}
	}
	return nil, NewSekaiUnknownClientException(response.StatusCode(), string(response.Body()))
}

func (c *SekaiClient) CallApi(ctx context.Context, path string, method string, data any, params map[string]any) (*resty.Response, error) {
	uri := fmt.Sprintf("%s/api/%s", c.ServerInfo.ApiUrl, path)

	template, err := uritemplates.Parse(uri)
	if err != nil {
		return nil, err
	}
	values := map[string]interface{}{
		"userId": c.Account.GetUserId(),
	}

	url, err := template.Expand(values)
	if err != nil {
		return nil, err
	}

	c.Logger.Infof("%s server account #%s %s %s",
		strings.ToUpper(string(c.ServerInfo.Server)),
		c.Account.GetUserId(),
		method,
		url,
	)

	cli := *c.Session
	if c.Proxy != "" {
		cli.SetProxy(c.Proxy)
	}
	req := *cli.R()
	req.SetContext(ctx)
	req.Header.Set("X-Request-ID", uuid.New().String())
	req.
		AddRetryCondition(func(response *resty.Response, err error) bool {
			transport, cliErr := cli.Transport()
			if cliErr != nil || transport.Proxy == nil {
				return true
			}

			var reqErr *netUrl.Error
			if errors.As(err, &reqErr) {
				var netError *net.OpError
				if errors.As(reqErr.Err, &netError) {
					if netError.Op == "proxyconnect" {
						c.Logger.Warnf("net error: proxy: %v", reqErr.Err)
					} else {
						c.Logger.Warnf("net error: %v", reqErr.Err)
					}
					return false // assume that this is proxy server error
				} else {
					if strings.Contains(reqErr.Err.Error(), "EOF") {
						c.Logger.Warnf("request error: %v (most probably proxy issue)", reqErr.Err)
						return false
					}
					c.Logger.Warnf("request error: %v", reqErr.Err)
					return true
				}
			}
			return false
		})

	for k, v := range params {
		req.SetQueryParam(k, fmt.Sprintf("%v", v))
	}
	if data != nil {
		packedData, err := Pack(data, c.ServerInfo.Server)
		if err != nil {
			return nil, err
		}
		req.SetBody(packedData)
	}

	response, err := req.Execute(strings.ToUpper(method), url)
	if err != nil {
		return nil, err
	}

	return response, nil
}

func (c *SekaiClient) Get(ctx context.Context, path string, params map[string]any) (*resty.Response, error) {
	return c.CallApi(ctx, path, "GET", nil, params)
}

func (c *SekaiClient) Post(ctx context.Context, path string, data any, params map[string]any) (*resty.Response, error) {
	return c.CallApi(ctx, path, "POST", data, params)
}

func (c *SekaiClient) Put(ctx context.Context, path string, data any, params map[string]any) (*resty.Response, error) {
	return c.CallApi(ctx, path, "PUT", data, params)
}

func (c *SekaiClient) Delete(ctx context.Context, path string, params map[string]any) (*resty.Response, error) {
	return c.CallApi(ctx, path, "DELETE", nil, params)
}

func (c *SekaiClient) Patch(ctx context.Context, path string, data any, params map[string]any) (*resty.Response, error) {
	return c.CallApi(ctx, path, "PATCH", data, params)
}

func (c *SekaiClient) Close() error {
	if c.Session != nil {
		c.Session = nil
	}
	return nil
}
