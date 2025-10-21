package config

import (
	harukiLogger "haruki-sekai-api/logger"
	"haruki-sekai-api/utils"
	"os"

	"gopkg.in/yaml.v3"
)

type RedisConfig struct {
	Host     string `yaml:"host"`
	Port     int    `yaml:"port"`
	Password string `yaml:"password"`
}

type BackendConfig struct {
	Host          string   `yaml:"host"`
	Port          int      `yaml:"port"`
	SSL           bool     `yaml:"ssl"`
	SSLCert       string   `yaml:"ssl_cert"`
	SSLKey        string   `yaml:"ssl_key"`
	LogLevel      string   `yaml:"log_level"`
	MainLogFile   string   `yaml:"main_log_file"`
	AccessLog     string   `yaml:"access_log"`
	AccessLogPath string   `yaml:"access_log_path"`
	AllowCORS     []string `yaml:"allow_cors"`
}

type GormLoggerConfig struct {
	Level                     string `yaml:"level"`
	SlowThreshold             string `yaml:"slow_threshold,omitempty"`
	IgnoreRecordNotFoundError bool   `yaml:"ignore_record_not_found_error,omitempty"`
	Colorful                  bool   `yaml:"colorful,omitempty"`
}

type GormNamingConfig struct {
	TablePrefix   string `yaml:"table_prefix,omitempty"`
	SingularTable bool   `yaml:"singular_table,omitempty"`
}

type GormConfig struct {
	Dialect                                  string           `yaml:"dialect"`
	DSN                                      string           `yaml:"dsn"`
	MaxOpenConns                             int              `yaml:"max_open_conns,omitempty"`
	MaxIdleConns                             int              `yaml:"max_idle_conns,omitempty"`
	ConnMaxLifetime                          string           `yaml:"conn_max_lifetime,omitempty"`
	PrepareStmt                              bool             `yaml:"prepare_stmt,omitempty"`
	DisableForeignKeyConstraintWhenMigrating bool             `yaml:"disable_fk_migrate,omitempty"`
	Logger                                   GormLoggerConfig `yaml:"logger"`
	Naming                                   GormNamingConfig `yaml:"naming"`
}

type ServerConfig struct {
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
}

type Config struct {
	Proxy   string                                         `yaml:"proxy"`
	Redis   RedisConfig                                    `yaml:"redis"`
	Backend BackendConfig                                  `yaml:"backend"`
	Gorm    GormConfig                                     `yaml:"gorm"`
	Servers map[utils.HarukiSekaiServerRegion]ServerConfig `yaml:"servers"`
}

var Cfg Config

func init() {
	logger := harukiLogger.NewLogger("ConfigLoader", "DEBUG", nil)
	f, err := os.Open("haruki-sekai-configs.yaml")
	if err != nil {
		logger.Errorf("Failed to open config file: %v", err)
		os.Exit(1)
	}
	defer f.Close()

	decoder := yaml.NewDecoder(f)
	if err := decoder.Decode(&Cfg); err != nil {
		logger.Errorf("Failed to parse config: %v", err)
		os.Exit(1)
	}
}
