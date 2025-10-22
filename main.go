package main

import (
	"fmt"
	"io"
	"os"

	"haruki-sekai-api/api"
	"haruki-sekai-api/config"
	harukiLogger "haruki-sekai-api/utils/logger"

	"github.com/gofiber/fiber/v2"
	"github.com/gofiber/fiber/v2/middleware/logger"
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
		defer logFile.Close()
	}
	mainLogger := harukiLogger.NewLogger("Main", config.Cfg.Backend.LogLevel, loggerWriter)
	mainLogger.Infof("========================= Haruki Sekai API %s =========================", config.Version)
	mainLogger.Infof("Powered By Haruki Dev Team")
	if err := api.InitAPIUtils(config.Cfg); err != nil {
		mainLogger.Errorf("failed to initialize API utils: %v", err)
		os.Exit(1)
	}
	app := fiber.New(fiber.Config{
		BodyLimit: 30 * 1024 * 1024,
	})

	if config.Cfg.Backend.AccessLog != "" {
		logCfg := logger.Config{Format: config.Cfg.Backend.AccessLog}
		if config.Cfg.Backend.AccessLogPath != "" {
			accessLogFile, err := os.OpenFile(config.Cfg.Backend.AccessLogPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
			if err != nil {
				mainLogger.Errorf("failed to open access log file: %v", err)
				os.Exit(1)
			}
			defer accessLogFile.Close()
			logCfg.Output = accessLogFile
		}
		app.Use(logger.New(logCfg))
	}

	api.RegisterRoutes(app)

	addr := fmt.Sprintf("%s:%d", config.Cfg.Backend.Host, config.Cfg.Backend.Port)
	if config.Cfg.Backend.SSL {
		if err := app.ListenTLS(addr, config.Cfg.Backend.SSLCert, config.Cfg.Backend.SSLKey); err != nil {
			mainLogger.Errorf("failed to start HTTPS server: %v", err)
			os.Exit(1)
		}
	} else {
		if err := app.Listen(addr); err != nil {
			mainLogger.Errorf("failed to start HTTP server: %v", err)
			os.Exit(1)
		}
	}
}
