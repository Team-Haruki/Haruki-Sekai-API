package client

import (
	"context"
	"fmt"
)

func (c *SekaiClient) Login(ctx context.Context) (any, error) {
	loginJson, err := c.Account.Dump()
	if err != nil {
		return nil, err
	}

	reqData, err := Pack(loginJson, c.ServerInfo.Server)
	if err != nil {
		return nil, err
	}

	var url, method string
	if _, ok := c.Account.(*SekaiAccountCP); ok {
		url = fmt.Sprintf("%s/api/user/%s/auth?refreshUpdatedResources=False", c.ServerInfo.ApiUrl, c.Account.GetUserId())
		method = "PUT"
	} else {
		url = fmt.Sprintf("%s/api/user/auth", c.ServerInfo.ApiUrl)
		method = "POST"
	}

	response, err := c.CallApi(ctx, url, method, reqData, nil)
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

		retData, err := UnpackInto[LoginResponse](response.Body(), c.ServerInfo.Server)
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

		c.Logger.Infof("Login successful. Server %s, User ID: %s", c.ServerInfo.Server, c.Account.GetUserId())
		return retData, nil
	default:
		c.Logger.Errorf("Login failed. Status code: %d, Response: %s", response.StatusCode(), string(response.Body()))
		return nil, NewSekaiUnknownClientException(response.StatusCode(), string(response.Body()))
	}
}

func (c *SekaiClient) GetMySekaiImage(ctx context.Context, path string) ([]byte, error) {
	// TODO: implement me
	panic("implement me")
}

func (c *SekaiClient) GetMasterData(ctx context.Context) (any, error) {
	// TODO: implement me
	panic("implement me")
}
