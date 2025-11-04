package api

import "github.com/gofiber/fiber/v3"

func RegisterRoutes(app *fiber.App) {
	registerHarukiSekaiAPIRoutes(app)
	registerHarukiSekaiImageRoutes(app)
}
