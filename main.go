package main

import (
	"fmt"
	"io"
	"os"

	"haruki-sekai-api/api"
	"haruki-sekai-api/config"
	harukiLogger "haruki-sekai-api/utils/logger"

	"github.com/bytedance/sonic"
	"github.com/gofiber/fiber/v3"
	"github.com/gofiber/fiber/v3/middleware/logger"
)

func main() {
	var logFile *os.File
	var loggerWriter io.Writer = os.Stdout
	if config.Cfg.Backend.MainLogFile != "" {
		var err error
		logFile, err = os.OpenFile(config.Cfg.Backend.MainLogFile, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
		if err != nil {
			mainLogger := harukiLogger.NewLogger("Main", config.Cfg.Backend.LogLevel, os.Stdout)
			mainLogger.Errorf("failed to open main log file: %v", err)
			os.Exit(1)
		}
		loggerWriter = io.MultiWriter(os.Stdout, logFile)
		defer func(logFile *os.File) {
			_ = logFile.Close()
		}(logFile)
	}
	mainLogger := harukiLogger.NewLogger("Main", config.Cfg.Backend.LogLevel, loggerWriter)
	mainLogger.Infof("========================= Haruki Sekai API %s =========================", config.Version)
	mainLogger.Infof("Powered By Haruki Dev Team")
	if err := api.InitAPIUtils(config.Cfg); err != nil {
		mainLogger.Errorf("failed to initialize API utils: %v", err)
		os.Exit(1)
	}
	app := fiber.New(fiber.Config{
		BodyLimit:   30 * 1024 * 1024,
		JSONEncoder: sonic.Marshal,
		JSONDecoder: sonic.Unmarshal,
		ProxyHeader: config.Cfg.Backend.ProxyHeader,
		TrustProxy:  config.Cfg.Backend.EnableTrustProxy,
		TrustProxyConfig: fiber.TrustProxyConfig{
			Proxies: config.Cfg.Backend.TrustProxies,
		},
	})

	if config.Cfg.Backend.AccessLog != "" {
		logCfg := logger.Config{Format: config.Cfg.Backend.AccessLog}
		if config.Cfg.Backend.AccessLogPath != "" {
			accessLogFile, err := os.OpenFile(config.Cfg.Backend.AccessLogPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
			if err != nil {
				mainLogger.Errorf("failed to open access log file: %v", err)
				os.Exit(1)
			}
			defer func(accessLogFile *os.File) {
				_ = accessLogFile.Close()
			}(accessLogFile)
			logCfg.Stream = accessLogFile
		}
		app.Use(logger.New(logCfg))
	}
	api.RegisterRoutes(app)
	appConfig := fiber.ListenConfig{
		DisableStartupMessage: true,
	}
	addr := fmt.Sprintf("%s:%d", config.Cfg.Backend.Host, config.Cfg.Backend.Port)
	if config.Cfg.Backend.SSL {
		mainLogger.Infof("SSL enabled, starting HTTPS server at %s", addr)
		appConfig.CertFile = config.Cfg.Backend.SSLCert
		appConfig.CertKeyFile = config.Cfg.Backend.SSLKey
		if err := app.Listen(addr, appConfig); err != nil {
			mainLogger.Errorf("failed to start HTTPS server: %v", err)
			os.Exit(1)
		}
	} else {
		mainLogger.Infof("Starting HTTP server at %s", addr)
		if err := app.Listen(addr, appConfig); err != nil {
			mainLogger.Errorf("failed to start HTTP server: %v", err)
			os.Exit(1)
		}
	}
}
