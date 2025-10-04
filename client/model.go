package client

import (
	"encoding/json"
	"fmt"
	"haruki-sekai-api/utils"
)

type SekaiAccountInterface interface {
	SetupAccount(userId string, deviceId string, token string)
	GetUserId() string
	SetUserId(userId string)
	GetDeviceId() string
	GetToken() string
	Dump() ([]byte, error)
}

func NewSekaiAccount[T SekaiAccountInterface](userId string, deviceId string, token string) T {
	var inst T
	inst.SetupAccount(userId, deviceId, token)
	return inst
}

type SekaiAccountCommonBase struct {
	UserId   string
	DeviceID string
}

type SekaiAccountCP struct {
	SekaiAccountCommonBase
	credential string
}

func (s *SekaiAccountCP) SetupAccount(userId string, deviceId string, token string) {
	s.UserId = userId
	s.DeviceID = deviceId
	s.credential = token
}
func (s *SekaiAccountCP) GetUserId() string       { return s.UserId }
func (s *SekaiAccountCP) SetUserId(userId string) { s.UserId = userId }
func (s *SekaiAccountCP) GetDeviceId() string     { return s.DeviceID }
func (s *SekaiAccountCP) GetToken() string        { return s.credential }
func (s *SekaiAccountCP) Dump() ([]byte, error) {
	data := struct {
		UserId   string `json:"userId"`
		DeviceID string `json:"deviceId"`
		Token    string `json:"credential"`
	}{
		UserId:   s.UserId,
		DeviceID: s.DeviceID,
		Token:    s.credential,
	}

	dump, err := json.Marshal(data)
	if err != nil {
		return nil, err
	}
	return dump, nil
}

type SekaiAccountNuverse struct {
	SekaiAccountCommonBase
	accessToken string
}

func (s *SekaiAccountNuverse) SetupAccount(userId string, deviceId string, token string) {
	s.UserId = userId
	s.DeviceID = deviceId
	s.accessToken = token
}
func (s *SekaiAccountNuverse) GetUserId() string       { return s.UserId }
func (s *SekaiAccountNuverse) SetUserId(userId string) { s.UserId = userId }
func (s *SekaiAccountNuverse) GetDeviceId() string     { return s.DeviceID }
func (s *SekaiAccountNuverse) GetToken() string        { return s.accessToken }
func (s *SekaiAccountNuverse) Dump() ([]byte, error) {
	data := struct {
		UserId   string `json:"userId"`
		DeviceID string `json:"deviceId"`
		Token    string `json:"accessToken"`
	}{
		UserId:   s.UserId,
		DeviceID: s.DeviceID,
		Token:    s.accessToken,
	}

	dump, err := json.Marshal(data)
	if err != nil {
		return nil, err
	}
	return dump, nil
}

type SekaiServerInfo struct {
	Server               utils.SekaiRegion
	ApiUrl               string
	NuverseMasterDataUrl string
	RequireCookies       bool
	Headers              map[string]string
	Enabled              bool
	AESKey               string
	AESIV                string
}

type HarukiAssetUpdaterInfo struct {
	Url           string
	Authorization string
}

type HarukiAppHashSourceType string

const (
	HarukiAppHashSourceTypeFile HarukiAppHashSourceType = "file"
	HarukiAppHashSourceTypeUrl  HarukiAppHashSourceType = "url"
)

func ParseHarukiAppHashSourceType(s string) (HarukiAppHashSourceType, error) {
	switch HarukiAppHashSourceType(s) {
	case HarukiAppHashSourceTypeFile,
		HarukiAppHashSourceTypeUrl:
		return HarukiAppHashSourceType(s), nil
	default:
		return "", fmt.Errorf("invalid app hash source type: %s", s)
	}
}

type HarukiAppHashSource struct {
	SourceType HarukiAppHashSourceType
	Dir        string
	Url        string
}

func NewHarukiAppHashSource(sourceType HarukiAppHashSourceType) (*HarukiAppHashSource, error) {
	inst := &HarukiAppHashSource{
		SourceType: HarukiAppHashSourceType(sourceType),
	}
	return inst, nil
}

type HarukiAppInfo struct {
	AppVersion string
	AppHash    string
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
