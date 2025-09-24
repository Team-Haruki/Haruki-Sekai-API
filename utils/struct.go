package utils

type SekaiRegion string

const (
	SekaiRegionJP SekaiRegion = "jp"
	SekaiRegionEN SekaiRegion = "en"
	SekaiRegionTW SekaiRegion = "tw"
	SekaiRegionKR SekaiRegion = "kr"
	SekaiRegionCN SekaiRegion = "cn"
)

type HarukiAppHashSourceType string

const (
	HarukiAppHashSourceTypeFile HarukiAppHashSourceType = "file"
	HarukiAppHashSourceTypeUrl  HarukiAppHashSourceType = "url"
)

type HarukiAppHashSource struct {
	Type HarukiAppHashSourceType `json:"type"`
	Dir  string                  `json:"dir,omitempty"`
	URL  string                  `json:"url,omitempty"`
}

type HarukiAppInfo struct {
	AppVersion string `json:"app_version"`
	AppHash    string `json:"app_hash"`
}
