package client

import (
	"context"
	"encoding/base64"
	"fmt"
	"net/url"
	"strings"
)

func (c *SekaiClient) Login(ctx context.Context) (any, error) {
	loginJson, err := c.Account.Dump()
	if err != nil {
		return nil, err
	}

	reqData, err := c.Cryptor.Pack(loginJson)
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

	response, err := c.CallAPI(ctx, loginURL, method, reqData, nil)
	if err != nil {
		return nil, err
	}

	parsedStatusCode, err := ParseSekaiApiHttpStatus(response.StatusCode())
	if err != nil {
		return nil, NewSekaiUnknownClientException(response.StatusCode(), string(response.Body()))
	}

	switch parsedStatusCode {
	case SekaiApiHttpStatusGameUpgrade:
		c.Logger.Warnf("Game upgrade required. (Current version: %s)", c.Session.Header.Get("X-App-Version"))
		return nil, NewUpgradeRequiredError()
	case SekaiApiHttpStatusUnderMaintenance:
		return nil, NewUnderMaintenanceError()
	case SekaiApiHttpStatusOk:
		type LoginResponse struct {
			SessionToken     string `msgpack:"sessionToken"`
			DataVersion      string `msgpack:"dataVersion"`
			AssetVersion     string `msgpack:"assetVersion"`
			UserRegistration struct {
				UserID string `msgpack:"userId"`
			} `msgpack:"userRegistration"`
		}

		retData, err := UnpackInto[LoginResponse](c.Cryptor, response.Body())
		if err != nil {
			c.Logger.Errorf("Unpack login response error : %v", err)
			return nil, err
		}
		if _, ok := c.Account.(*SekaiAccountNuverse); ok {
			if retData.UserRegistration.UserID == "" {
				return nil, fmt.Errorf("invalid login response: missing user ID")
			}
			c.Account.SetUserId(retData.UserRegistration.UserID)
		}

		if retData.SessionToken == "" || retData.DataVersion == "" || retData.AssetVersion == "" {
			return nil, fmt.Errorf("invalid login response: missing required fields")
		}
		c.Session.Header.Set("X-Session-Token", retData.SessionToken)
		c.Session.Header.Set("X-Data-Version", retData.DataVersion)
		c.Session.Header.Set("X-Asset-Version", retData.AssetVersion)

		c.Logger.Infof("Login successful. Server %s, User ID: %s", c.Server, c.Account.GetUserId())
		return retData, nil
	default:
		c.Logger.Errorf("Login failed. Status code: %d, Response: %s", response.StatusCode(), string(response.Body()))
		return nil, NewSekaiUnknownClientException(response.StatusCode(), string(response.Body()))
	}
}

func (c *SekaiClient) GetCPMySekaiImage(path string) ([]byte, error) {
	ctx := context.Background()
	pathNew := strings.TrimPrefix(path, "/")
	url := fmt.Sprintf("%s/image/mysekai-photo/%s", c.ServerConfig.APIURL, pathNew)
	cli := *c.Session
	if c.Proxy != "" {
		cli.SetProxy(c.Proxy)
	}
	req := *cli.R()
	req.SetContext(ctx)
	resp, err := req.Get(url)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode() != 200 {
		return nil, fmt.Errorf("unexpected status %d fetching %s", resp.StatusCode(), url)
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
	m, ok := respAny.(map[string]any)
	if !ok {
		return nil, fmt.Errorf("unexpected response type: %T", respAny)
	}
	b64, ok := m["thumbnail"].(string)
	if !ok || b64 == "" {
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

func (c *SekaiClient) GetNuverseMasterData(cdnVersion int) (any, error) {
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
