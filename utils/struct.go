package utils

import "fmt"

type HarukiSekaiServerRegion string

const (
	HarukiSekaiServerRegionJP HarukiSekaiServerRegion = "jp"
	HarukiSekaiServerRegionEN HarukiSekaiServerRegion = "en"
	HarukiSekaiServerRegionTW HarukiSekaiServerRegion = "tw"
	HarukiSekaiServerRegionKR HarukiSekaiServerRegion = "kr"
	HarukiSekaiServerRegionCN HarukiSekaiServerRegion = "cn"
)

func ParseSekaiServerRegion(s string) (HarukiSekaiServerRegion, error) {
	switch HarukiSekaiServerRegion(s) {
	case HarukiSekaiServerRegionJP,
		HarukiSekaiServerRegionEN,
		HarukiSekaiServerRegionTW,
		HarukiSekaiServerRegionKR,
		HarukiSekaiServerRegionCN:
		return HarukiSekaiServerRegion(s), nil
	default:
		return "", fmt.Errorf("invalid server region: %s", s)
	}
}

type HarukiSekaiAPIEndpointType string // APIType

const (
	HarukiSekaiAPIEndpointTypeAPI   HarukiSekaiAPIEndpointType = "api"
	HarukiSekaiAPIEndpointTypeImage HarukiSekaiAPIEndpointType = "image"
)

func ParseAPIEndpointType(s string) (HarukiSekaiAPIEndpointType, error) {
	switch HarukiSekaiAPIEndpointType(s) {
	case HarukiSekaiAPIEndpointTypeAPI, HarukiSekaiAPIEndpointTypeImage:
		return HarukiSekaiAPIEndpointType(s), nil
	default:
		return "", fmt.Errorf("invalid endpoint type: %s", s)
	}
}

type HarukiSekaiAppHashSourceType string

const (
	HarukiSekaiAppHashSourceTypeFile HarukiSekaiAppHashSourceType = "file"
	HarukiSekaiAppHashSourceTypeUrl  HarukiSekaiAppHashSourceType = "url"
)

type HarukiSekaiAppHashSource struct {
	Type HarukiSekaiAppHashSourceType `json:"type"`
	Dir  string                       `json:"dir,omitempty"`
	URL  string                       `json:"url,omitempty"`
}

type HarukiSekaiAppInfo struct {
	AppVersion string `json:"app_version"`
	AppHash    string `json:"app_hash"`
}
