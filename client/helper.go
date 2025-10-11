package client

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"sync"
	"time"

	"github.com/go-resty/resty/v2"
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
		client := resty.New()
		client.SetTimeout(10 * time.Second)
		if proxy != "" {
			client.SetProxy(proxy)
		}

		resp, err := client.R().
			SetContext(ctx).
			SetHeader("Accept", "*/*").
			SetHeader("User-Agent", "ProductName/134 CFNetwork/1408.0.4 Darwin/22.5.0").
			SetHeader("Connection", "keep-alive").
			SetHeader("Accept-Language", "zh-CN,zh-Hans;q=0.9").
			SetHeader("Accept-Encoding", "gzip, deflate, br").
			SetHeader("X-Unity-Version", "2022.3.21f1").
			Post(h.url)

		if err != nil {
			lastErr = err
			time.Sleep(1 * time.Second)
			continue
		}

		if resp.StatusCode() == 200 {
			cookie := resp.Header().Get("Set-Cookie")
			h.cookies = cookie
			return cookie, nil
		} else {
			lastErr = errors.New("failed to fetch cookies")
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
