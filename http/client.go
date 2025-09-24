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

func NewClient(proxy string, timeout time.Duration) *Client {
	return &Client{Proxy: proxy, Timeout: timeout, client: &fasthttp.Client{}}
}

func (c *Client) init() error {
	if c.client != nil {
		return nil
	}
	c.client = &fasthttp.Client{}
	if c.Proxy == "" {
		return nil
	}

	proxyURL, err := url.Parse(c.Proxy)
	if err != nil {
		return fmt.Errorf("invalid proxy url: %v", err)
	}
	switch proxyURL.Scheme {
	case "http", "https":
		c.client.Dial = fasthttpproxy.FasthttpHTTPDialer(proxyURL.Host)
	case "socks5":
		c.client.Dial = fasthttpproxy.FasthttpSocksDialer(proxyURL.Host)
	default:
		return fmt.Errorf("unsupported proxy scheme: %s", proxyURL.Scheme)
	}
	return nil
}

func (c *Client) Request(ctx context.Context, method, uri string, headers map[string]string, body []byte) (int, map[string]string, []byte, error) {
	if err := c.init(); err != nil {
		return 0, nil, nil, err
	}

	req := fasthttp.AcquireRequest()
	resp := fasthttp.AcquireResponse()
	defer fasthttp.ReleaseRequest(req)
	defer fasthttp.ReleaseResponse(resp)

	req.Header.SetMethod(method)
	req.SetRequestURI(uri)
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	if len(body) > 0 {
		req.SetBody(body)
	}

	timeout := c.Timeout
	if timeout == 0 {
		timeout = 15 * time.Second
	}

	errCh := make(chan error, 1)
	go func() { errCh <- c.client.DoTimeout(req, resp, timeout) }()

	select {
	case <-ctx.Done():
		return 0, nil, nil, ctx.Err()
	case err := <-errCh:
		if err != nil {
			return 0, nil, nil, err
		}
	}

	respHeaders := make(map[string]string, resp.Header.Len())
	for k, v := range resp.Header.All() {
		respHeaders[string(append([]byte(nil), k...))] = string(append([]byte(nil), v...))
	}
	return resp.StatusCode(), respHeaders, append([]byte(nil), resp.Body()...), nil
}
