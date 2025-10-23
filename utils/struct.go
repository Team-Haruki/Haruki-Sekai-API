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

type HarukiSekaiServerConfig struct {
	Enabled                  bool              `yaml:"enabled,omitempty"`
	MasterDir                string            `yaml:"master_dir,omitempty"`
	VersionPath              string            `yaml:"version_path,omitempty"`
	AccountDir               string            `yaml:"account_dir,omitempty"`
	APIURL                   string            `yaml:"api_url"`
	NuverseMasterDataURL     string            `yaml:"nuverse_master_data_url,omitempty"`
	NuverseStructureFilePath string            `yaml:"nuverse_structure_file_path,omitempty"`
	RequireCookies           bool              `yaml:"require_cookies,omitempty"`
	Headers                  map[string]string `yaml:"headers,omitempty"`
	AESKeyHex                string            `yaml:"aes_key_hex,omitempty"`
	AESIVHex                 string            `yaml:"aes_iv_hex,omitempty"`
	EnableMasterUpdater      bool              `yaml:"enable_master_updater,omitempty"`
	MasterUpdaterCron        string            `yaml:"master_updater_cron,omitempty"`
	EnableAppHashUpdater     bool              `yaml:"enable_app_hash_updater,omitempty"`
	AppHashUpdaterCron       string            `yaml:"app_hash_updater_cron,omitempty"`
}

type HarukiAssetUpdaterInfo struct {
	URL           string `yaml:"url"`
	Authorization string `yaml:"authorization,omitempty"`
}

type HarukiSekaiLoginResponse struct {
	SessionToken         string   `msgpack:"sessionToken"`
	DataVersion          string   `msgpack:"dataVersion"`
	AssetVersion         string   `msgpack:"assetVersion"`
	AssetHash            string   `msgpack:"assetHash"`
	SuiteMasterSplitPath []string `msgpack:"suiteMasterSplitPath"`
	CDNVersion           int      `msgpack:"cdnVersion"`
	UserRegistration     struct {
		UserID any `msgpack:"userId"`
	} `msgpack:"userRegistration"`
}
