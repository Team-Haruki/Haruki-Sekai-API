package updater

import (
	"strconv"
	"strings"
)

func CompareVersion(newVersion, currentVersion string) bool {
	newParts := strings.Split(newVersion, ".")
	currentParts := strings.Split(currentVersion, ".")

	for i := 0; i < len(newParts); i++ {
		newPart, err1 := strconv.Atoi(newParts[i])
		currentPart, err2 := strconv.Atoi(currentParts[i])
		if err1 != nil || err2 != nil {
			continue
		}
		if newPart > currentPart {
			return true
		} else if newPart < currentPart {
			return false
		}
	}
	return false
}
