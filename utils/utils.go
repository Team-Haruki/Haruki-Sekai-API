package utils

import (
	"strconv"

	"github.com/hashicorp/go-version"
)

func CompareVersion(newVersion, currentVersion string) (bool, error) {
	v1, err := version.NewVersion(newVersion)
	if err != nil {
		return false, err
	}
	v2, err := version.NewVersion(currentVersion)
	if err != nil {
		return false, err
	}
	return v1.GreaterThan(v2), nil
}

func GetString(m map[string]any, key string) string {
	if v, ok := m[key]; ok {
		if s, ok := v.(string); ok {
			return s
		}
	}
	return ""
}

func GetInt(m map[string]any, key string) int {
	if v, ok := m[key]; ok {
		switch val := v.(type) {
		case int:
			return val
		case float64:
			return int(val)
		case string:
			if i, err := strconv.Atoi(val); err == nil {
				return i
			}
		}
	}
	return 0
}
