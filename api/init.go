package api

import (
	"context"
	"fmt"
	"haruki-sekai-api/client"
	"haruki-sekai-api/config"
	"haruki-sekai-api/utils"
	"haruki-sekai-api/utils/apphash"
	"haruki-sekai-api/utils/git"
	harukiLogger "haruki-sekai-api/utils/logger"
	"log"
	"os"
	"strings"
	"time"

	"github.com/go-co-op/gocron/v2"
	"github.com/redis/go-redis/v9"
	"gorm.io/driver/mysql"
	"github.com/redis/go-redis/v9/maintnotifications"
	"gorm.io/driver/postgres"
	"gorm.io/driver/sqlite"
	"gorm.io/driver/sqlserver"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"
	"gorm.io/gorm/schema"
)

var (
	harukiGit                    *git.HarukiGitUpdater
	HarukiSekaiManagers          map[utils.HarukiSekaiServerRegion]*client.SekaiClientManager
	HarukiSekaiRedis             *redis.Client
	HarukiSekaiUserDB            *gorm.DB
	HarukiSekaiUserJWTSigningKey *string
	harukiSchedulerLogger        *harukiLogger.Logger
)

func gormLoggerFromConfig(lc config.GormLoggerConfig) logger.Interface {
	var lvl logger.LogLevel
	switch strings.ToLower(lc.Level) {
	case "silent":
		lvl = logger.Silent
	case "error":
		lvl = logger.Error
	case "warn", "warning":
		lvl = logger.Warn
	default:
		lvl = logger.Info
	}
	cfg := logger.Config{
		SlowThreshold:             0,
		Colorful:                  lc.Colorful,
		IgnoreRecordNotFoundError: lc.IgnoreRecordNotFoundError,
		LogLevel:                  lvl,
	}
	return logger.New(log.New(os.Stdout, "", log.LstdFlags), cfg)
}

func openGorm(cfg config.GormConfig) (*gorm.DB, error) {
	if !cfg.Enabled {
		return nil, nil
	}
	if cfg.Dialect == "" || cfg.DSN == "" {
		return nil, nil
	}
	gCfg := &gorm.Config{
		PrepareStmt:                              cfg.PrepareStmt,
		DisableForeignKeyConstraintWhenMigrating: cfg.DisableForeignKeyConstraintWhenMigrating,
		NamingStrategy: schema.NamingStrategy{
			TablePrefix:   cfg.Naming.TablePrefix,
			SingularTable: cfg.Naming.SingularTable,
		},
		Logger: gormLoggerFromConfig(cfg.Logger),
	}
	var (
		db  *gorm.DB
		err error
	)
	switch strings.ToLower(cfg.Dialect) {
	case "mysql":
		db, err = gorm.Open(mysql.Open(cfg.DSN), gCfg)
	case "postgres", "postgresql":
		db, err = gorm.Open(postgres.Open(cfg.DSN), gCfg)
	case "sqlite", "sqlite3":
		db, err = gorm.Open(sqlite.Open(cfg.DSN), gCfg)
	case "sqlserver", "mssql":
		db, err = gorm.Open(sqlserver.Open(cfg.DSN), gCfg)
	default:
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if sqlDB, err := db.DB(); err == nil {
		if cfg.MaxOpenConns > 0 {
			sqlDB.SetMaxOpenConns(cfg.MaxOpenConns)
		}
		if cfg.MaxIdleConns > 0 {
			sqlDB.SetMaxIdleConns(cfg.MaxIdleConns)
		}
		if cfg.ConnMaxLifetime != "" {
			if d, err := time.ParseDuration(cfg.ConnMaxLifetime); err == nil {
				sqlDB.SetConnMaxLifetime(d)
			}
		}
	}
	return db, nil
}

func openRedis(cfg config.RedisConfig) (*redis.Client, error) {
	if !cfg.Enabled {
		return nil, nil
	}
	if cfg.Host == "" || cfg.Port == 0 {
		return nil, nil
	}
	addr := fmt.Sprintf("%s:%d", cfg.Host, cfg.Port)
	rdb := redis.NewClient(&redis.Options{
		Addr:     addr,
		Password: cfg.Password,
		DB:       0,
		MaintNotificationsConfig: &maintnotifications.Config{
			Mode: maintnotifications.ModeDisabled,
		},
	})
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := rdb.Ping(ctx).Err(); err != nil {
		return nil, err
	}
	return rdb, nil
}

func initDatabase(cfg config.Config) error {
	db, err := openGorm(cfg.Gorm)
	if err != nil {
		return err
	}
	HarukiSekaiUserDB = db

	if HarukiSekaiUserDB != nil {
		if err := HarukiSekaiUserDB.AutoMigrate(&SekaiUser{}, &SekaiUserServer{}); err != nil {
			return err
		}
	}

	rdb, err := openRedis(cfg.Redis)
	if err != nil {
		return err
	}
	HarukiSekaiRedis = rdb
	return nil
}

func initSekaiManagers(cfg config.Config, harukiGit *git.HarukiGitUpdater) map[utils.HarukiSekaiServerRegion]*client.SekaiClientManager {
	sekaiManager := make(map[utils.HarukiSekaiServerRegion]*client.SekaiClientManager)
	for server, serverConfig := range cfg.Servers {
		if serverConfig.Enabled {
			sekaiManager[server] = client.NewSekaiClientManager(server, serverConfig, cfg.AssetUpdaterServers, harukiGit, cfg.Proxy, cfg.JPSekaiCookieURL)
			_ = sekaiManager[server].Init()
		}
	}
	return sekaiManager
}

func registerMasterUpdaters(cfg config.Config, sekaiManager map[utils.HarukiSekaiServerRegion]*client.SekaiClientManager, sch gocron.Scheduler) error {
	for server, serverConfig := range cfg.Servers {
		if !serverConfig.Enabled || !serverConfig.EnableMasterUpdater || serverConfig.MasterUpdaterCron == "" {
			continue
		}
		mgr := sekaiManager[server]
		if mgr == nil {
			continue
		}
		_, err := sch.NewJob(
			gocron.CronJob(serverConfig.MasterUpdaterCron, true),
			gocron.NewTask(func(srv utils.HarukiSekaiServerRegion, m *client.SekaiClientManager) {
				defer func() {
					if r := recover(); r != nil {
						harukiSchedulerLogger.Infof("%s CheckSekaiMasterUpdate panic: %v", strings.ToUpper(string(srv)), r)
					}
				}()
				m.CheckSekaiMasterUpdate()
			}, server, mgr),
		)
		if err != nil {
			return fmt.Errorf("register updater for %s failed: %w", server, err)
		}
		harukiSchedulerLogger.Infof("%s sekai updater registered cron: %s", strings.ToUpper(string(server)), serverConfig.MasterUpdaterCron)
	}
	return nil
}

func registerAppHashUpdaters(cfg config.Config, sch gocron.Scheduler) error {
	for server, serverConfig := range cfg.Servers {
		if !serverConfig.Enabled || !serverConfig.EnableAppHashUpdater || serverConfig.AppHashUpdaterCron == "" {
			continue
		}
		updater := apphash.NewAppHashUpdater(cfg.AppHashSources, server, &serverConfig.VersionPath)
		_, err := sch.NewJob(
			gocron.CronJob(serverConfig.AppHashUpdaterCron, true),
			gocron.NewTask(func(srv utils.HarukiSekaiServerRegion, u *apphash.HarukiSekaiAppHashUpdater) {
				defer func() {
					if r := recover(); r != nil {
						harukiSchedulerLogger.Infof("%s CheckAppVersion panic: %v", strings.ToUpper(string(srv)), r)
					}
				}()
				u.CheckAppVersion()
			}, server, updater),
		)
		if err != nil {
			return fmt.Errorf("register apphash updater for %s failed: %w", server, err)
		}
		harukiSchedulerLogger.Infof("%s apphash updater registered cron: %s", strings.ToUpper(string(server)), serverConfig.AppHashUpdaterCron)
	}
	return nil
}

func InitAPIUtils(cfg config.Config) error {
	if cfg.Git.Enabled {
		harukiGit = git.NewHarukiGitUpdater(cfg.Git.Username, cfg.Git.Email, cfg.Git.Password, cfg.Proxy)
	}

	if err := initDatabase(cfg); err != nil {
		return err
	}

	sekaiManager := initSekaiManagers(cfg, harukiGit)
	HarukiSekaiManagers = sekaiManager

	sch, err := gocron.NewScheduler(gocron.WithLocation(time.Local))
	if err != nil {
		return err
	}
	harukiSchedulerLogger = harukiLogger.NewLogger("HarukiSekaiUpdaterScheduler", "DEBUG", nil)

	if err := registerMasterUpdaters(cfg, sekaiManager, sch); err != nil {
		return err
	}

	if err := registerAppHashUpdaters(cfg, sch); err != nil {
		return err
	}

	sch.Start()

	if cfg.Backend.SekaiUserJWTSigningKey != "" {
		HarukiSekaiUserJWTSigningKey = &cfg.Backend.SekaiUserJWTSigningKey
	}
	return nil
}
