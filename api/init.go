package api

import (
	"context"
	"fmt"
	"haruki-sekai-api/client"
	"haruki-sekai-api/config"
	"haruki-sekai-api/utils"
	"haruki-sekai-api/utils/git"
	"log"
	"os"
	"strings"
	"time"

	"github.com/redis/go-redis/v9"
	"gorm.io/driver/mysql"
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
	})
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := rdb.Ping(ctx).Err(); err != nil {
		return nil, err
	}
	return rdb, nil
}

func InitAPIUtils(cfg config.Config) error {
	sekaiManager := make(map[utils.HarukiSekaiServerRegion]*client.SekaiClientManager)
	if cfg.Git.Enabled {
		harukiGit = git.NewHarukiGitUpdater(cfg.Git.Username, cfg.Git.Email, cfg.Git.Password, cfg.Proxy)
	}

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

	for server, serverConfig := range cfg.Servers {
		if serverConfig.Enabled {
			sekaiManager[server] = client.NewSekaiClientManager(server, serverConfig, cfg.AssetUpdaterServers, harukiGit, cfg.Proxy)
		}
	}
	HarukiSekaiManagers = sekaiManager

	if cfg.Backend.SekaiUserJWTSigningKey != "" {
		HarukiSekaiUserJWTSigningKey = &cfg.Backend.SekaiUserJWTSigningKey
	}
	return nil
}
