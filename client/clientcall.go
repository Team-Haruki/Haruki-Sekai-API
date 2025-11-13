package client

import (
	"context"
	"encoding/base64"
	"fmt"
	"haruki-sekai-api/utils"
	"strconv"
	"strings"
	"time"

	"github.com/google/uuid"
)

func (c *SekaiClient) prepareLoginRequest() (string, string) {
	if _, ok := c.Account.(*SekaiAccountCP); ok {
		loginURL := fmt.Sprintf("%s/api/user/%s/auth?refreshUpdatedResources=False", c.ServerConfig.APIURL, c.Account.GetUserId())
		return loginURL, "PUT"
	}
	loginURL := fmt.Sprintf("%s/api/user/auth", c.ServerConfig.APIURL)
	return loginURL, "POST"
}

func validateLoginResponse(retData *utils.HarukiSekaiLoginResponse) error {
	if retData.SessionToken == "" || retData.DataVersion == "" || retData.AssetVersion == "" {
		return fmt.Errorf("invalid login response: missing required fields")
	}
	return nil
}

func extractUserID(userID any) (string, error) {
	switch v := userID.(type) {
	case string:
		return v, nil
	case int64:
		return strconv.FormatInt(v, 10), nil
	case uint64:
		return strconv.FormatUint(v, 10), nil
	case int:
		return strconv.Itoa(v), nil
	case float64:
		return strconv.FormatInt(int64(v), 10), nil
	default:
		return "", fmt.Errorf("unexpected userId type %T", v)
	}
}

func (c *SekaiClient) handleNuverseUserID(retData *utils.HarukiSekaiLoginResponse) error {
	if _, ok := c.Account.(*SekaiAccountNuverse); !ok {
		return nil
	}

	uidStr, err := extractUserID(retData.UserRegistration.UserID)
	if err != nil {
		return fmt.Errorf("invalid login response: %w", err)
	}
	if uidStr == "" {
		return fmt.Errorf("invalid login response: missing user ID")
	}
	c.Account.SetUserId(uidStr)
	return nil
}

func (c *SekaiClient) updateHeadersFromLogin(retData *utils.HarukiSekaiLoginResponse) {
	c.HeaderLock.Lock()
	oldToken := c.Headers["X-Session-Token"]
	c.Headers["X-Session-Token"] = retData.SessionToken
	c.Headers["X-Data-Version"] = retData.DataVersion
	c.Headers["X-Asset-Version"] = retData.AssetVersion
	c.Logger.Debugf("account #%s login updated session token (old: %s..., new: %s...)",
		c.Account.GetUserId(),
		truncateString(oldToken, 80),
		truncateString(retData.SessionToken, 80))
	c.HeaderLock.Unlock()
}

func (c *SekaiClient) Login(ctx context.Context) (*utils.HarukiSekaiLoginResponse, error) {
	loginMsgpack, err := c.Account.Dump()
	if err != nil {
		return nil, err
	}

	loginURL, method := c.prepareLoginRequest()

	encBody, err := c.Cryptor.Pack(loginMsgpack)
	if err != nil {
		c.Logger.Errorf("login pack error: %v", err)
		return nil, err
	}

	ctxTimeout, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()

	req := c.Session.R()
	req.SetContext(ctxTimeout)
	c.HeaderLock.Lock()
	req.SetHeaders(c.Headers)
	c.HeaderLock.Unlock()
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

		if err := validateLoginResponse(retData); err != nil {
			return nil, err
		}

		if err := c.handleNuverseUserID(retData); err != nil {
			return nil, err
		}

		c.updateHeadersFromLogin(retData)

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
	c.APILock.Lock()
	defer c.APILock.Unlock()
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
