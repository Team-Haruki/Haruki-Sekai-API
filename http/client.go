package http

import (
	"context"
	"fmt"
	"net/url"
	"time"

	"github.com/valyala/fasthttp"
	"github.com/valyala/fasthttp/fasthttpproxy"
)

type Client struct {
	Proxy   string
	Timeout time.Duration
	client  *fasthttp.Client
}

func (c *Client) Request(ctx context.Context, method, uri string, headers map[string]string, body []byte) (status int, respHeaders map[string]string, respBody []byte, err error) {
	req := fasthttp.AcquireRequest()
	resp := fasthttp.AcquireResponse()
	defer fasthttp.ReleaseRequest(req)
	defer fasthttp.ReleaseResponse(resp)

	req.Header.SetMethod(method)
	req.SetRequestURI(uri)
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	if body != nil {
		req.SetBody(body)
	}

	if c.client == nil {
		c.client = &fasthttp.Client{}
		if c.Proxy != "" {
			proxyURL, err := url.Parse(c.Proxy)
			if err != nil {
				return 0, nil, nil, fmt.Errorf("invalid proxy url: %v", err)
			}
			switch proxyURL.Scheme {
			case "http":
				c.client.Dial = fasthttpproxy.FasthttpHTTPDialer(proxyURL.Host)
			case "https":
				c.client.Dial = fasthttpproxy.FasthttpHTTPDialer(proxyURL.Host)
			case "socks5":
				c.client.Dial = fasthttpproxy.FasthttpSocksDialer(proxyURL.Host)
			default:
				return 0, nil, nil, fmt.Errorf("unsupported proxy scheme: %s", proxyURL.Scheme)
			}
		}
	}

	timeout := c.Timeout
	if timeout == 0 {
		timeout = 15 * time.Second
	}
	errChan := make(chan error, 1)
	go func() {
		errChan <- c.client.DoTimeout(req, resp, timeout)
	}()

	select {
	case <-ctx.Done():
		return 0, nil, nil, ctx.Err()
	case err := <-errChan:
		if err != nil {
			return 0, nil, nil, err
		}
	}

	status = resp.StatusCode()
	respHeaders = make(map[string]string)
	for k, v := range resp.Header.All() {
		respHeaders[string(append([]byte(nil), k...))] = string(append([]byte(nil), v...))
	}
	respBody = append([]byte(nil), resp.Body()...)
	return status, respHeaders, respBody, nil
}
