package config

import (
	harukiLogger "haruki-sekai-api/logger"
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

type SekaiClientConfig struct {
	ENServerAPIHost              string            `yaml:"en_server_api_host"`
	ENServerAESKey               string            `yaml:"en_server_aes_key"`
	ENServerAESIV                string            `yaml:"en_server_aes_iv"`
	JPServerAPIHost              string            `yaml:"jp_server_api_host"`
	TWServerAPIHost              string            `yaml:"tw_server_api_host"`
	KRServerAPIHost              string            `yaml:"kr_server_api_host"`
	CNServerAPIHost              string            `yaml:"cn_server_api_host"`
	OtherServerAESKey            string            `yaml:"other_server_aes_key"`
	OtherServerAESIV             string            `yaml:"other_server_aes_iv"`
	JPServerInheritToken         string            `yaml:"jp_server_inherit_token"`
	ENServerInheritToken         string            `yaml:"en_server_inherit_token"`
	JPServerAppVersionUrl        string            `yaml:"jp_server_app_version_url"`
	ENServerAppVersionUrl        string            `yaml:"en_server_app_version_url"`
	JPServerInheritClientHeaders map[string]string `yaml:"jp_server_inherit_client_headers"`
	ENServerInheritClientHeaders map[string]string `yaml:"en_server_inherit_client_headers"`
	SuiteRemoveKeys              []string          `yaml:"suite_remove_keys"`
}

type Config struct {
	Proxy       string            `yaml:"proxy"`
	Redis       RedisConfig       `yaml:"redis"`
	Backend     BackendConfig     `yaml:"backend"`
	SekaiClient SekaiClientConfig `yaml:"sekai_client"`
}

var Cfg Config

func init() {
	logger := harukiLogger.NewLogger("ConfigLoader", "DEBUG", nil)
	f, err := os.Open("haruki-suite-configs.yaml")
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
