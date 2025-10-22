package api

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	"haruki-sekai-api/utils"

	"github.com/gofiber/fiber/v2"
	"github.com/golang-jwt/jwt/v5"
	"gorm.io/gorm"
)

func resolveServerFromCtx(c *fiber.Ctx) (utils.HarukiSekaiServerRegion, error) {
	s := strings.ToLower(c.Params("server"))
	if s == "" {
		return "", fmt.Errorf("missing server")
	}
	return utils.ParseSekaiServerRegion(s)
}

func validateUserTokenMiddleware() fiber.Handler {
	return func(c *fiber.Ctx) error {
		if HarukiSekaiUserDB == nil {
			return c.Next()
		}

		tokenStr := c.Get("X-Haruki-Sekai-Token")
		if tokenStr == "" {
			return fiber.NewError(fiber.StatusUnauthorized, "Missing token")
		}
		if HarukiSekaiUserJWTSigningKey == nil || *HarukiSekaiUserJWTSigningKey == "" {
			return fiber.NewError(fiber.StatusUnauthorized, "JWT secret not configured")
		}

		claims := jwt.MapClaims{}
		token, err := jwt.ParseWithClaims(tokenStr, claims, func(t *jwt.Token) (interface{}, error) {
			if t.Method.Alg() != jwt.SigningMethodHS256.Alg() {
				return nil, fmt.Errorf("unexpected signing method: %s", t.Method.Alg())
			}
			return []byte(*HarukiSekaiUserJWTSigningKey), nil
		})
		if err != nil || !token.Valid {
			return fiber.NewError(fiber.StatusUnauthorized, "Invalid token")
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

		if HarukiSekaiRedis != nil {
			ctx, cancel := context.WithTimeout(context.Background(), time.Second)
			defer cancel()
			redisKey := fmt.Sprintf("haruki_sekai_api:%s:%s", uid, server)
			if val, err := HarukiSekaiRedis.Get(ctx, redisKey).Result(); err == nil && val != "" {
				c.Locals("sekaiUser", SekaiUser{ID: uid, Credential: credential, Remark: ""})
				return c.Next()
			}
		}

		var user SekaiUser
		if err := HarukiSekaiUserDB.Where("id = ?", uid).Take(&user).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return fiber.NewError(fiber.StatusUnauthorized, "User not found")
			}
			return fiber.NewError(fiber.StatusInternalServerError, "Database error")
		}
		if user.Credential != credential {
			return fiber.NewError(fiber.StatusUnauthorized, "Invalid credential")
		}

		var us SekaiUserServer
		if err := HarukiSekaiUserDB.Where("user_id = ? AND server = ?", uid, server).Take(&us).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return fiber.NewError(fiber.StatusForbidden, "Not authorized for this server")
			}
			return fiber.NewError(fiber.StatusInternalServerError, "Database error")
		}

		if HarukiSekaiRedis != nil {
			ctx, cancel := context.WithTimeout(context.Background(), time.Second)
			defer cancel()
			redisKey := fmt.Sprintf("haruki_sekai_api:%s:%s", uid, server)
			if err := HarukiSekaiRedis.Set(ctx, redisKey, "1", 12*time.Hour).Err(); err != nil {
			}
		}

		c.Locals("sekaiUser", SekaiUser{ID: uid, Credential: credential, Remark: user.Remark})
		return c.Next()
	}
}
