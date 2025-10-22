package client

import (
	"context"
	"encoding/base64"
	"fmt"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/google/uuid"
)

func (c *SekaiClient) Login(ctx context.Context) (any, error) {
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
		type LoginResponse struct {
			SessionToken     string `msgpack:"sessionToken"`
			DataVersion      string `msgpack:"dataVersion"`
			AssetVersion     string `msgpack:"assetVersion"`
			UserRegistration struct {
				UserID any `msgpack:"userId"`
			} `msgpack:"userRegistration"`
		}

		retData, err := UnpackInto[LoginResponse](c.Cryptor, resp.Body())
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

func (c *SekaiClient) GetCPMasterData(paths []string) (map[string]any, error) {
	master := make(map[string]any)
	ctx := context.Background()
	for _, rawPath := range paths {
		if rawPath == "" {
			continue
		}
		path := strings.TrimPrefix(rawPath, "/")
		resp, err := c.Get(ctx, path, nil)
		if err != nil {
			return nil, err
		}
		unpacked, err := c.Cryptor.Unpack(resp.Body())
		if err != nil {
			c.Logger.Errorf("unpack master part failed: path=%s, err=%v", rawPath, err)
			return nil, err
		}
		part, ok := unpacked.(map[string]any)
		if !ok {
			return nil, fmt.Errorf("unexpected master data type at path %s", rawPath)
		}
		for k, v := range part {
			master[k] = v
		}
	}
	return master, nil
}

func (c *SekaiClient) GetNuverseMasterData(cdnVersion int) (map[string]any, error) {
	ctx := context.Background()
	u := fmt.Sprintf("%s/master-data-%d.info", c.ServerConfig.NuverseMasterDataURL, cdnVersion)
	parsed, err := url.Parse(u)
	if err != nil {
		return nil, err
	}
	host := parsed.Hostname()
	cli := *c.Session
	if c.Proxy != "" {
		cli.SetProxy(c.Proxy)
	}
	req := *cli.R()
	req.SetContext(ctx)
	if host != "" {
		req.SetHeader("Host", host)
	}
	resp, err := req.Get(u)
	if err != nil {
		return nil, err
	}
	unpacked, err := c.Cryptor.Unpack(resp.Body())
	if err != nil {
		return nil, fmt.Errorf("unpack nuverse master info failed: %w", err)
	}
	masterMap, ok := unpacked.(map[string]any)
	if !ok {
		return nil, fmt.Errorf("unexpected nuverse master info type")
	}
	restored, err := NuverseMasterRestorer(masterMap, c.ServerConfig.NuverseStructureFilePath)
	if err != nil {
		return nil, err
	}
	return restored, nil
}
