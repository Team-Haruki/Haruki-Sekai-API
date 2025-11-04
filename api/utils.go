package api

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	"haruki-sekai-api/utils"

	"github.com/gofiber/fiber/v3"
	"github.com/golang-jwt/jwt/v5"
	"gorm.io/gorm"
)

func resolveServerFromCtx(c fiber.Ctx) (utils.HarukiSekaiServerRegion, error) {
	s := strings.ToLower(c.Params("server"))
	if s == "" {
		return "", fmt.Errorf("missing server")
	}
	return utils.ParseSekaiServerRegion(s)
}

func parseJWTToken(tokenStr string) (*jwt.Token, jwt.MapClaims, error) {
	if HarukiSekaiUserJWTSigningKey == nil || *HarukiSekaiUserJWTSigningKey == "" {
		return nil, nil, fmt.Errorf("JWT secret not configured")
	}

	claims := jwt.MapClaims{}
	token, err := jwt.ParseWithClaims(tokenStr, claims, func(t *jwt.Token) (interface{}, error) {
		if t.Method.Alg() != jwt.SigningMethodHS256.Alg() {
			return nil, fmt.Errorf("unexpected signing method: %s", t.Method.Alg())
		}
		return []byte(*HarukiSekaiUserJWTSigningKey), nil
	})
	if err != nil || !token.Valid {
		return nil, nil, fmt.Errorf("invalid token")
	}

	return token, claims, nil
}

func checkRedisCache(uid, server string) bool {
	if HarukiSekaiRedis == nil {
		return false
	}

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	redisKey := fmt.Sprintf("haruki_sekai_api:%s:%s", uid, server)
	if val, err := HarukiSekaiRedis.Get(ctx, redisKey).Result(); err == nil && val != "" {
		return true
	}
	return false
}

func validateUserInDB(uid, credential, server string) (*SekaiUser, error) {
	var user SekaiUser
	if err := HarukiSekaiUserDB.Where("id = ?", uid).Take(&user).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, fiber.NewError(fiber.StatusUnauthorized, "User not found")
		}
		return nil, fiber.NewError(fiber.StatusInternalServerError, "Database error")
	}
	if user.Credential != credential {
		return nil, fiber.NewError(fiber.StatusUnauthorized, "Invalid credential")
	}

	var us SekaiUserServer
	if err := HarukiSekaiUserDB.Where("user_id = ? AND server = ?", uid, server).Take(&us).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, fiber.NewError(fiber.StatusForbidden, "Not authorized for this server")
		}
		return nil, fiber.NewError(fiber.StatusInternalServerError, "Database error")
	}

	return &user, nil
}

func cacheUserInRedis(uid, server string) {
	if HarukiSekaiRedis == nil {
		return
	}

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	redisKey := fmt.Sprintf("haruki_sekai_api:%s:%s", uid, server)
	_ = HarukiSekaiRedis.Set(ctx, redisKey, "1", 12*time.Hour).Err()
}

func validateUserTokenMiddleware() fiber.Handler {
	return func(c fiber.Ctx) error {
		if HarukiSekaiUserDB == nil {
			return c.Next()
		}

		tokenStr := c.Get("X-Haruki-Sekai-Token")
		if tokenStr == "" {
			return fiber.NewError(fiber.StatusUnauthorized, "Missing token")
		}

		_, claims, err := parseJWTToken(tokenStr)
		if err != nil {
			return fiber.NewError(fiber.StatusUnauthorized, err.Error())
		}

		uid, _ := claims["uid"].(string)
		credential, _ := claims["credential"].(string)
		if uid == "" || credential == "" {
			return fiber.NewError(fiber.StatusUnauthorized, "Invalid token payload")
		}

		region, err := resolveServerFromCtx(c)
		if err != nil {
			return fiber.NewError(fiber.StatusBadRequest, err.Error())
		}
		server := string(region)

		if checkRedisCache(uid, server) {
			c.Locals("sekaiUser", SekaiUser{ID: uid, Credential: credential, Remark: ""})
			return c.Next()
		}

		user, err := validateUserInDB(uid, credential, server)
		if err != nil {
			return err
		}

		cacheUserInRedis(uid, server)

		c.Locals("sekaiUser", SekaiUser{ID: uid, Credential: credential, Remark: user.Remark})
		return c.Next()
	}
}
