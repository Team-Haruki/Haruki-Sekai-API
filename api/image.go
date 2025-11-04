package api

import (
	"fmt"
	"haruki-sekai-api/utils"
	"regexp"
	"strings"

	"github.com/gofiber/fiber/v3"
)

func getMySekaiImage(c fiber.Ctx) error {
	region, err := utils.ParseSekaiServerRegion(strings.ToLower(c.Params("server")))
	if err != nil {
		return fiber.NewError(fiber.StatusBadRequest, err.Error())
	}

	param1 := c.Params("param1")
	param2 := c.Params("param2")

	hex64 := regexp.MustCompile(`^[a-f0-9]{64}$`)
	digits := regexp.MustCompile(`^\d+$`)

	mgr, ok := HarukiSekaiManagers[region]
	if !ok || mgr == nil {
		return fiber.NewError(fiber.StatusServiceUnavailable, "server not initialized")
	}

	switch region {
	case utils.HarukiSekaiServerRegionJP, utils.HarukiSekaiServerRegionEN:
		if !hex64.MatchString(param1) || !hex64.MatchString(param2) {
			return fiber.NewError(fiber.StatusBadRequest, "invalid path format for colorful palette servers")
		}
		combined := param1 + "/" + param2
		img, err := mgr.GetCPMySekaiImage(combined)
		if err != nil {
			return fiber.NewError(fiber.StatusBadGateway, fmt.Sprintf("fetch image failed: %v", err))
		}
		c.Set("Content-Type", "image/png")
		return c.Send(img)
	default:
		if !digits.MatchString(param1) || !digits.MatchString(param2) {
			return fiber.NewError(fiber.StatusBadRequest, "invalid path format for nuverse servers")
		}
		img, err := mgr.GetNuverseMySekaiImage(param1, param2)
		if err != nil {
			return fiber.NewError(fiber.StatusBadGateway, fmt.Sprintf("fetch image failed: %v", err))
		}
		c.Set("Content-Type", "image/png")
		return c.Send(img)
	}
}

func registerHarukiSekaiImageRoutes(app *fiber.App) {
	image := app.Group("/image/:server", validateUserTokenMiddleware())

	image.Get("/mysekai/:param1/:param2", getMySekaiImage)
}
