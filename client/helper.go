package client

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/url"
	"os"
	"sync"
	"time"
)

type SekaiCookieHelper struct {
	url     string
	cookies string
	mu      sync.Mutex
}

func (h *SekaiCookieHelper) GetCookies(ctx context.Context, proxy string) (string, error) {
	h.mu.Lock()
	defer h.mu.Unlock()

	var lastErr error
	for attempt := 0; attempt < 4; attempt++ {
		req, err := http.NewRequestWithContext(ctx, http.MethodPost,
			h.url, nil)
		if err != nil {
			return "", err
		}

		req.Header.Set("Accept", "*/*")
		req.Header.Set("User-Agent", "ProductName/134 CFNetwork/1408.0.4 Darwin/22.5.0")
		req.Header.Set("Connection", "keep-alive")
		req.Header.Set("Accept-Language", "zh-CN,zh-Hans;q=0.9")
		req.Header.Set("Accept-Encoding", "gzip, deflate, br")
		req.Header.Set("X-Unity-Version", "2022.3.21f1")

		transport := &http.Transport{}
		if proxy != "" {
			proxyURL, err := url.Parse(proxy)
			if err != nil {
				return "", err
			}
			transport.Proxy = http.ProxyURL(proxyURL)
		}

		client := &http.Client{
			Timeout:   10 * time.Second,
			Transport: transport,
		}

		resp, err := client.Do(req)
		if err != nil {
			lastErr = err
			time.Sleep(1 * time.Second)
			continue
		}

		if resp.StatusCode == http.StatusOK {
			cookie := resp.Header.Get("Set-Cookie")
			h.cookies = cookie
			resp.Body.Close()
			return cookie, nil
		} else {
			lastErr = errors.New("failed to fetch cookies")
			resp.Body.Close()
			time.Sleep(1 * time.Second)
		}
	}
	return "", lastErr
}

type SekaiVersionHelper struct {
	versionFilePath string
	AppVersion      string
	AppHash         string
	DataVersion     string
	AssetVersion    string
	mu              sync.Mutex
}

func (h *SekaiVersionHelper) GetAppVersion() error {
	h.mu.Lock()
	defer h.mu.Unlock()

	data, err := os.ReadFile(h.versionFilePath)
	if err != nil {
		return err
	}

	var parsed map[string]string
	if err := json.Unmarshal(data, &parsed); err != nil {
		return err
	}

	h.AppVersion = parsed["appVersion"]
	h.AppHash = parsed["appHash"]
	h.DataVersion = parsed["dataVersion"]
	h.AssetVersion = parsed["assetVersion"]

	return nil
}
