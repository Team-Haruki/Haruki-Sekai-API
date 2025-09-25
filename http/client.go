package http

import (
	"context"
	"time"

	"github.com/go-resty/resty/v2"
)

type Client struct {
	Proxy   string
	Timeout time.Duration
	client  *resty.Client
}

func NewClient(proxy string, timeout time.Duration) *Client {
	client := &Client{Proxy: proxy, Timeout: timeout}
	err := client.init()
	if err != nil {
		panic(err)
	}
	return client
}

func (c *Client) init() error {
	if c.client != nil {
		return nil
	}
	c.client = resty.New()
	if c.Timeout != 0 {
		c.client.SetTimeout(c.Timeout)
	} else {
		c.client.SetTimeout(15 * time.Second)
	}
	if c.Proxy != "" {
		c.client.SetProxy(c.Proxy)
	}
	return nil
}

func (c *Client) Request(ctx context.Context, method, uri string, headers map[string]string, body []byte) (int, map[string]string, []byte, error) {
	resp, err := c.client.R().
		SetContext(ctx).
		SetHeaders(headers).
		SetBody(body).
		Execute(method, uri)
	if err != nil {
		return 0, nil, nil, err
	}

	respHeaders := make(map[string]string, len(resp.Header()))
	for k, v := range resp.Header() {
		if len(v) > 0 {
			respHeaders[k] = v[0]
		}
	}
	return resp.StatusCode(), respHeaders, resp.Body(), nil
}
