package client

import (
	"fmt"
	"haruki-sekai-api/utils"

	"github.com/vmihailenco/msgpack/v5"
)

type SekaiAccountInterface interface {
	SetupAccount(userId int64, deviceId string, token string)
	GetUserId() int64
	SetUserId(userId int64)
	GetDeviceId() string
	GetToken() string
	Dump() ([]byte, error)
}

func NewSekaiAccount[T SekaiAccountInterface](userId int64, deviceId string, token string) T {
	var inst T
	inst.SetupAccount(userId, deviceId, token)
	return inst
}

type SekaiAccountCommonBase struct {
	UserId   int64  `json:"userId"`
	DeviceID string `json:"deviceId,omitempty"`
}

type SekaiAccountCP struct {
	SekaiAccountCommonBase
	Credential string `json:"credential"`
}

func (s *SekaiAccountCP) SetupAccount(userId int64, deviceId string, token string) {
	s.UserId = userId
	s.DeviceID = deviceId
	s.Credential = token
}
func (s *SekaiAccountCP) GetUserId() int64       { return s.UserId }
func (s *SekaiAccountCP) SetUserId(userId int64) { s.UserId = userId }
func (s *SekaiAccountCP) GetDeviceId() string    { return s.DeviceID }
func (s *SekaiAccountCP) GetToken() string       { return s.Credential }
func (s *SekaiAccountCP) Dump() ([]byte, error) {
	var deviceID *string
	if s.DeviceID != "" {
		deviceID = &s.DeviceID
	}
	payload := map[string]any{
		"deviceId":        deviceID,
		"credential":      s.Credential,
		"authTriggerType": "normal",
	}
	return msgpack.Marshal(payload)
}

type SekaiAccountNuverse struct {
	SekaiAccountCommonBase
	AccessToken string `json:"accessToken"`
}

func (s *SekaiAccountNuverse) SetupAccount(userId int64, deviceId string, token string) {
	s.UserId = userId
	s.DeviceID = deviceId
	s.AccessToken = token
}
func (s *SekaiAccountNuverse) GetUserId() int64       { return s.UserId }
func (s *SekaiAccountNuverse) SetUserId(userId int64) { s.UserId = userId }
func (s *SekaiAccountNuverse) GetDeviceId() string    { return s.DeviceID }
func (s *SekaiAccountNuverse) GetToken() string       { return s.AccessToken }
func (s *SekaiAccountNuverse) Dump() ([]byte, error) {
	data := struct {
		UserId   int64  `msgpack:"userId"`
		DeviceID string `msgpack:"deviceId"`
		Token    string `msgpack:"accessToken"`
	}{
		UserId:   s.UserId,
		DeviceID: s.DeviceID,
		Token:    s.AccessToken,
	}
	return msgpack.Marshal(data)
}

type SekaiApiHttpStatus int

const (
	SekaiApiHttpStatusOk               SekaiApiHttpStatus = 200
	SekaiApiHttpStatusClientError      SekaiApiHttpStatus = 400
	SekaiApiHttpStatusSessionError     SekaiApiHttpStatus = 403
	SekaiApiHttpStatusNotFound         SekaiApiHttpStatus = 404
	SekaiApiHttpStatusConflict         SekaiApiHttpStatus = 409
	SekaiApiHttpStatusGameUpgrade      SekaiApiHttpStatus = 426
	SekaiApiHttpStatusServerError      SekaiApiHttpStatus = 500
	SekaiApiHttpStatusUnderMaintenance SekaiApiHttpStatus = 503
)

func ParseSekaiApiHttpStatus(code int) (SekaiApiHttpStatus, error) {
	switch SekaiApiHttpStatus(code) {
	case SekaiApiHttpStatusOk,
		SekaiApiHttpStatusClientError,
		SekaiApiHttpStatusSessionError,
		SekaiApiHttpStatusNotFound,
		SekaiApiHttpStatusConflict,
		SekaiApiHttpStatusGameUpgrade,
		SekaiApiHttpStatusServerError,
		SekaiApiHttpStatusUnderMaintenance:
		return SekaiApiHttpStatus(code), nil
	default:
		return 0, fmt.Errorf("invalid http status code: %d", code)
	}
}

type HarukiSekaiAssetUpdaterPayload struct {
	Server       utils.HarukiSekaiServerRegion `json:"server"`
	AssetVersion string                        `json:"assetVersion"`
	AssetHash    string                        `json:"assetHash"`
}

type HarukiSekaiAPIFailedResponse struct {
	Result  string `json:"result"`
	Status  int    `json:"status"`
	Message string `json:"message"`
}
