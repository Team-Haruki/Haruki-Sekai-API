package api

import "github.com/gofiber/fiber/v2"

func RegisterRoutes(app *fiber.App) {
	registerHarukiSekaiAPIRoutes(app)
	registerHarukiSekaiImageRoutes(app)
}
