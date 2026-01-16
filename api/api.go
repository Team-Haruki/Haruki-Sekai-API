package api

import (
	"context"
	"fmt"
	"regexp"
	"strings"
	"time"

	"haruki-sekai-api/client"
	"haruki-sekai-api/utils"

	"github.com/gofiber/fiber/v3"
)

var digitsRe = regexp.MustCompile(`^\d+$`)

func getMgr(c fiber.Ctx) (utils.HarukiSekaiServerRegion, *client.SekaiClientManager, error) {
	region, err := utils.ParseSekaiServerRegion(strings.ToLower(c.Params("server")))
	if err != nil {
		return "", nil, fiber.NewError(fiber.StatusBadRequest, err.Error())
	}
	mgr, ok := HarukiSekaiManagers[region]
	if !ok || mgr == nil {
		return "", nil, fiber.NewError(fiber.StatusServiceUnavailable, "server not initialized")
	}
	return region, mgr, nil
}

func proxyGameAPI(c fiber.Ctx, path string, params map[string]any) error {
	_, mgr, err := getMgr(c)
	if err != nil {
		return err
	}
	ctx, cancel := context.WithTimeout(c.RequestCtx(), 45*time.Second)
	defer cancel()
	data, status, _ := mgr.GetGameAPI(ctx, path, params)
	return c.Status(status).JSON(data)
}

func getUserProfile(c fiber.Ctx) error {
	userID := c.Params("user_id")
	if userID == "" || !digitsRe.MatchString(userID) {
		return fiber.NewError(fiber.StatusBadRequest, "user_id must be numeric")
	}
	path := fmt.Sprintf("/user/{userId}/%s/profile", userID)
	return proxyGameAPI(c, path, nil)
}

func getSystem(c fiber.Ctx) error {
	return proxyGameAPI(c, "/system", nil)
}

func getInformation(c fiber.Ctx) error {
	return proxyGameAPI(c, "/information", nil)
}

func getEventRankingTop100(c fiber.Ctx) error {
	eventID := c.Params("event_id")
	if !digitsRe.MatchString(eventID) {
		return fiber.NewError(fiber.StatusBadRequest, "event_id must be numeric")
	}
	path := fmt.Sprintf("/user/{userId}/event/%s/ranking?rankingViewType=top100", eventID)
	return proxyGameAPI(c, path, nil)
}

func getEventRankingBorder(c fiber.Ctx) error {
	eventID := c.Params("event_id")
	if !digitsRe.MatchString(eventID) {
		return fiber.NewError(fiber.StatusBadRequest, "event_id must be numeric")
	}
	return proxyGameAPI(c, fmt.Sprintf("/event/%s/ranking-border", eventID), nil)
}

func registerHarukiSekaiAPIRoutes(app *fiber.App) {
	api := app.Group("/api/:server", validateUserTokenMiddleware())

	api.Get("/:user_id/profile", getUserProfile)
	api.Get("/system", getSystem)
	api.Get("/information", getInformation)
	api.Get("/event/:event_id/ranking-top100", getEventRankingTop100)
	api.Get("/event/:event_id/ranking-border", getEventRankingBorder)

}
