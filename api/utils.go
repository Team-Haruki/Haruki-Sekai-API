package api

import "haruki-sekai-api/utils"

type Request struct {
	APIType utils.HarukiSekaiAPIEndpointType
	Server  utils.HarukiSekaiServerRegion
	Path    string
	Query   map[string]string
}

func GetAPIRequest(apiType utils.HarukiSekaiAPIEndpointType, server utils.HarukiSekaiServerRegion, subPath string, query map[string]string) *Request {
	req := &Request{
		APIType: apiType,
		Server:  server,
		Path:    subPath,
		Query:   query,
	}
	return req
}
