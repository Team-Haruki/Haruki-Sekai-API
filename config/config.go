package config

import (
	"haruki-sekai-api/utils"
	harukiLogger "haruki-sekai-api/utils/logger"
	"os"

	"gopkg.in/yaml.v3"
)

type RedisConfig struct {
	Enabled  bool   `yaml:"enabled"`
	Host     string `yaml:"host"`
	Port     int    `yaml:"port"`
	Password string `yaml:"password"`
}

type BackendConfig struct {
	Host                   string   `yaml:"host"`
	Port                   int      `yaml:"port"`
	SSL                    bool     `yaml:"ssl"`
	SSLCert                string   `yaml:"ssl_cert"`
	SSLKey                 string   `yaml:"ssl_key"`
	LogLevel               string   `yaml:"log_level"`
	MainLogFile            string   `yaml:"main_log_file"`
	AccessLog              string   `yaml:"access_log"`
	AccessLogPath          string   `yaml:"access_log_path"`
	SekaiUserJWTSigningKey string   `yaml:"sekai_user_jwt_signing_key,omitempty"`
	EnableTrustProxy       bool     `yaml:"enable_trust_proxy"`
	TrustProxies           []string `yaml:"trusted_proxies"`
	ProxyHeader            string   `yaml:"proxy_header"`
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
	Enabled                                  bool             `yaml:"enabled"`
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

type GitConfig struct {
	Enabled  bool   `yaml:"enabled"`
	Username string `yaml:"username,omitempty"`
	Email    string `yaml:"email,omitempty"`
	Password string `yaml:"password,omitempty"`
}

type Config struct {
	Proxy               string                                                          `yaml:"proxy"`
	JPSekaiCookieURL    string                                                          `yaml:"jp_sekai_cookie_url"`
	Git                 GitConfig                                                       `yaml:"git"`
	Redis               RedisConfig                                                     `yaml:"redis"`
	Backend             BackendConfig                                                   `yaml:"backend"`
	Gorm                GormConfig                                                      `yaml:"gorm"`
	AppHashSources      []utils.HarukiSekaiAppHashSource                                `yaml:"apphash_sources"`
	AssetUpdaterServers []utils.HarukiAssetUpdaterInfo                                  `yaml:"asset_updater_servers"`
	Servers             map[utils.HarukiSekaiServerRegion]utils.HarukiSekaiServerConfig `yaml:"servers"`
}

var Version = "v5.0.0-dev"
var Cfg Config

func init() {
	logger := harukiLogger.NewLogger("ConfigLoader", "DEBUG", nil)
	f, err := os.Open("haruki-sekai-configs.yaml")
	if err != nil {
		logger.Errorf("Failed to open config file: %v", err)
		os.Exit(1)
	}
	defer func(f *os.File) {
		_ = f.Close()
	}(f)

	decoder := yaml.NewDecoder(f)
	if err := decoder.Decode(&Cfg); err != nil {
		logger.Errorf("Failed to parse config: %v", err)
		os.Exit(1)
	}
}
