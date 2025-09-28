package updater

import (
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
