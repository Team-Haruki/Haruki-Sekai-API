package api

import "fmt"

type ServerRegion string // APIServerRegion

const (
	ServerRegionJP ServerRegion = "jp"
	ServerRegionEN ServerRegion = "en"
	ServerRegionTW ServerRegion = "tw"
	ServerRegionKR ServerRegion = "kr"
	ServerRegionCN ServerRegion = "cn"
)

func ParseServerRegion(s string) (ServerRegion, error) {
	switch ServerRegion(s) {
	case ServerRegionJP,
		ServerRegionEN,
		ServerRegionTW,
		ServerRegionKR,
		ServerRegionCN:
		return ServerRegion(s), nil
	default:
		return "", fmt.Errorf("invalid server region: %s", s)
	}
}

type EndpointType string // APIType

const (
	EndpointTypeAPI   EndpointType = "api"
	EndpointTypeImage EndpointType = "image"
)

func ParseEndpointType(s string) (EndpointType, error) {
	switch EndpointType(s) {
	case EndpointTypeAPI, EndpointTypeImage:
		return EndpointType(s), nil
	default:
		return "", fmt.Errorf("invalid endpoint type: %s", s)
	}
}
