package utils

import (
	"fmt"

	"github.com/hashicorp/go-version"
	"github.com/iancoleman/orderedmap"
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

func GetString(om *orderedmap.OrderedMap, key string) string {
	if v, ok := om.Get(key); ok {
		switch t := v.(type) {
		case string:
			return t
		case fmt.Stringer:
			return t.String()
		case int:
			return fmt.Sprintf("%d", t)
		case int64:
			return fmt.Sprintf("%d", t)
		case float64:
			return fmt.Sprintf("%g", t)
		}
	}
	return ""
}

func GetInt(om *orderedmap.OrderedMap, key string) int {
	if v, ok := om.Get(key); ok {
		switch t := v.(type) {
		case int:
			return t
		case int64:
			return int(t)
		case float64:
			return int(t)
		case string:
			var n int
			_, err := fmt.Sscanf(t, "%d", &n)
			if err != nil {
				return 0
			}
			return n
		}
	}
	return 0
}
