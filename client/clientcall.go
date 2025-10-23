package client

import (
	"context"
	"encoding/base64"
	"fmt"
	"haruki-sekai-api/utils"
	"net/url"
	"strconv"
	"strings"
	"time"

	"sync"

	"github.com/google/uuid"
	"github.com/iancoleman/orderedmap"
	"golang.org/x/sync/errgroup"
)

func (c *SekaiClient) Login(ctx context.Context) (*utils.HarukiSekaiLoginResponse, error) {
	loginMsgpack, err := c.Account.Dump()
	if err != nil {
		return nil, err
	}

	var loginURL, method string
	if _, ok := c.Account.(*SekaiAccountCP); ok {
		loginURL = fmt.Sprintf("%s/api/user/%s/auth?refreshUpdatedResources=False", c.ServerConfig.APIURL, c.Account.GetUserId())
		method = "PUT"
	} else {
		loginURL = fmt.Sprintf("%s/api/user/auth", c.ServerConfig.APIURL)
		method = "POST"
	}

	encBody, err := c.Cryptor.Pack(loginMsgpack)
	if err != nil {
		c.Logger.Errorf("login pack error: %v", err)
		return nil, err
	}

	ctxTimeout, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()

	req := c.Session.R()
	req.SetContext(ctxTimeout)
	req.SetHeaders(c.Headers)
	req.Header.Set("X-Request-Id", uuid.New().String())
	req.SetBody(encBody)
	resp, err := req.Execute(method, loginURL)
	if err != nil {
		return nil, err
	}
	parsedStatusCode, err := ParseSekaiApiHttpStatus(resp.StatusCode())
	if err != nil {
		return nil, NewSekaiUnknownClientException(resp.StatusCode(), string(resp.Body()))
	}
	switch parsedStatusCode {
	case SekaiApiHttpStatusGameUpgrade:
		c.Logger.Warnf("Game upgrade required. (Current version: %s)", c.Headers["X-App-Version"])
		return nil, NewUpgradeRequiredError()
	case SekaiApiHttpStatusUnderMaintenance:
		return nil, NewUnderMaintenanceError()
	case SekaiApiHttpStatusOk:

		retData, err := UnpackInto[utils.HarukiSekaiLoginResponse](c.Cryptor, resp.Body())
		if err != nil {
			c.Logger.Errorf("Unpack login response error : %v", err)
			return nil, err
		}

		if retData.SessionToken == "" || retData.DataVersion == "" || retData.AssetVersion == "" {
			return nil, fmt.Errorf("invalid login response: missing required fields")
		}

		if _, ok := c.Account.(*SekaiAccountNuverse); ok {
			var uidStr string
			switch v := retData.UserRegistration.UserID.(type) {
			case string:
				uidStr = v
			case int64:
				uidStr = strconv.FormatInt(v, 10)
			case uint64:
				uidStr = strconv.FormatUint(v, 10)
			case int:
				uidStr = strconv.Itoa(v)
			case float64:
				uidStr = strconv.FormatInt(int64(v), 10)
			default:
				return nil, fmt.Errorf("invalid login response: unexpected userId type %T", v)
			}
			if uidStr == "" {
				return nil, fmt.Errorf("invalid login response: missing user ID")
			}
			c.Account.SetUserId(uidStr)
		}

		c.Headers["X-Session-Token"] = retData.SessionToken
		c.Headers["X-Data-Version"] = retData.DataVersion
		c.Headers["X-Asset-Version"] = retData.AssetVersion

		c.Logger.Infof("Login successfully, User ID: %s", c.Account.GetUserId())
		return retData, nil
	default:
		if unpacked, decErr := c.Cryptor.Unpack(resp.Body()); decErr == nil {
			c.Logger.Warnf("Login failed. Status code: %d, Decrypted: %#v", resp.StatusCode(), unpacked)
		} else {
			c.Logger.Warnf("Login failed. Status code: %d, Raw len=%d", resp.StatusCode(), len(resp.Body()))
		}
		return nil, NewSekaiUnknownClientException(resp.StatusCode(), string(resp.Body()))
	}
}

func (c *SekaiClient) GetCPMySekaiImage(path string) ([]byte, error) {
	ctx := context.Background()
	pathNew := strings.TrimPrefix(path, "/")
	imageURL := fmt.Sprintf("%s/image/mysekai-photo/%s", c.ServerConfig.APIURL, pathNew)
	cli := *c.Session
	if c.Proxy != "" {
		cli.SetProxy(c.Proxy)
	}
	req := *cli.R()
	req.SetContext(ctx)
	req.SetHeaders(c.Headers)
	resp, err := req.Get(imageURL)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode() != 200 {
		return nil, fmt.Errorf("unexpected status %d fetching %s", resp.StatusCode(), imageURL)
	}
	return resp.Body(), nil
}

func (c *SekaiClient) GetNuverseMySekaiImage(userID, index string) ([]byte, error) {
	ctx := context.Background()
	path := fmt.Sprintf("/user/%s/mysekai/photo/%s", userID, index)
	responseRaw, err := c.Get(ctx, path, nil)
	if err != nil {
		return nil, err
	}
	respAny, err := c.handleResponse(*responseRaw)
	if err != nil {
		return nil, err
	}

	ptr, ok := respAny.(*interface{})
	if !ok || ptr == nil {
		return nil, fmt.Errorf("unexpected response type: %T", respAny)
	}

	m, ok := (*ptr).(map[string]any)
	if !ok {
		return nil, fmt.Errorf("unexpected inner type: %T", *ptr)
	}

	b64, _ := m["thumbnail"].(string)
	if b64 == "" {
		return nil, fmt.Errorf("missing thumbnail base64 in response")
	}
	img, err := base64.StdEncoding.DecodeString(b64)
	if err != nil {
		return nil, fmt.Errorf("decode thumbnail base64 failed: %w", err)
	}
	return img, nil
}

func (c *SekaiClient) GetCPMasterData(paths []string) (*orderedmap.OrderedMap, error) {
	master := orderedmap.New()
	master.SetEscapeHTML(false)
	ctx := context.Background()

	var mu sync.Mutex
	eg, egCtx := errgroup.WithContext(ctx)
	sem := make(chan struct{}, 12)

	for _, rawPath := range paths {
		rp := rawPath
		if rp == "" {
			continue
		}
		eg.Go(func() error {
			select {
			case sem <- struct{}{}:
				defer func() { <-sem }()
			case <-egCtx.Done():
				return egCtx.Err()
			}

			p := rp
			if !strings.HasPrefix(p, "/") {
				p = "/" + p
			}

			resp, err := c.Get(egCtx, p, nil)
			if err != nil {
				return err
			}
			om, err := c.Cryptor.UnpackOrdered(resp.Body())
			if err != nil {
				return fmt.Errorf("unpack master part failed: path=%s, err=%w", rp, err)
			}
			if om == nil {
				return fmt.Errorf("unexpected master data: nil ordered map at path %s", rp)
			}

			mu.Lock()
			for _, k := range om.Keys() {
				if v, ok := om.Get(k); ok {
					master.Set(k, v)
				}
			}
			mu.Unlock()
			return nil
		})
	}

	if err := eg.Wait(); err != nil {
		return nil, err
	}
	return master, nil
}

func (c *SekaiClient) GetNuverseMasterData(cdnVersion int) (*orderedmap.OrderedMap, error) {
	start := time.Now()
	ctx := context.Background()

	u := fmt.Sprintf("%s/master-data-%d.info", c.ServerConfig.NuverseMasterDataURL, cdnVersion)
	parsed, err := url.Parse(u)
	if err != nil {
		c.Logger.Errorf("GetNuverseMasterData: url parse error: %v", err)
		return nil, err
	}
	host := parsed.Hostname()

	c.Logger.Debugf("GetNuverseMasterData: cdnVersion=%d url=%s host=%s proxy=%q", cdnVersion, u, host, c.Proxy)
	c.Logger.Debugf("GetNuverseMasterData: structurePath=%s", c.ServerConfig.NuverseStructureFilePath)
	c.Logger.Debugf("GetNuverseMasterData: client headers snapshot: %#v", c.Headers)

	cli := *c.Session
	if c.Proxy != "" {
		cli.SetProxy(c.Proxy)
	}
	req := *cli.R()
	req.SetContext(ctx)
	if host != "" {
		req.SetHeader("Host", host)
	}
	// Note: we currently don't copy game headers for this CDN call; logging the final request headers below.
	c.Logger.Debugf("GetNuverseMasterData: request headers => Host=%q, UA=%q", req.Header.Get("Host"), req.Header.Get("User-Agent"))

	resp, err := req.Get(u)
	if err != nil {
		c.Logger.Errorf("GetNuverseMasterData: request error: %v", err)
		return nil, err
	}
	if resp == nil {
		c.Logger.Errorf("GetNuverseMasterData: nil response")
		return nil, fmt.Errorf("nil response")
	}

	status := resp.StatusCode()
	body := resp.Body()
	ct := resp.Header().Get("Content-Type")
	cl := resp.Header().Get("Content-Length")
	c.Logger.Debugf("GetNuverseMasterData: resp status=%d content-type=%q content-length=%q body-len=%d", status, ct, cl, len(body))
	if len(body) > 0 {
		preview := 64
		if len(body) < preview {
			preview = len(body)
		}
		c.Logger.Debugf("GetNuverseMasterData: resp body hex preview=%x", body[:preview])
	}
	if status < 200 || status >= 300 {
		c.Logger.Warnf("GetNuverseMasterData: non-success status=%d", status)
	}

	masterOM, err := c.Cryptor.UnpackOrdered(body)
	if err != nil {
		c.Logger.Errorf("GetNuverseMasterData: unpack ordered failed: %v", err)
		return nil, fmt.Errorf("unpack nuverse master info failed: %w", err)
	}
	if masterOM == nil {
		c.Logger.Errorf("GetNuverseMasterData: unpack returned nil ordered map")
		return nil, fmt.Errorf("unexpected nuverse master info: nil ordered map")
	}

	c.Logger.Debugf("GetNuverseMasterData: unpack ok, keys=%d", len(masterOM.Keys()))
	if len(masterOM.Keys()) > 0 {
		keys := masterOM.Keys()
		max := 10
		if len(keys) < max {
			max = len(keys)
		}
		for i := 0; i < max; i++ {
			k := keys[i]
			if v, ok := masterOM.Get(k); ok {
				c.Logger.Debugf("  key[%d]=%s type=%T", i, k, v)
			}
		}
	}

	restored, err := NuverseMasterRestorer(masterOM, c.ServerConfig.NuverseStructureFilePath)
	if err != nil {
		c.Logger.Errorf("GetNuverseMasterData: NuverseMasterRestorer error: %v", err)
		return nil, err
	}
	c.Logger.Debugf("GetNuverseMasterData: restored keys=%d (elapsed=%s)", len(restored.Keys()), time.Since(start))
	if len(restored.Keys()) > 0 {
		keys := restored.Keys()
		max := 10
		if len(keys) < max {
			max = len(keys)
		}
		for i := 0; i < max; i++ {
			k := keys[i]
			if v, ok := restored.Get(k); ok {
				c.Logger.Debugf("  restored[%d]=%s type=%T", i, k, v)
			}
		}
	}
	return restored, nil
}
