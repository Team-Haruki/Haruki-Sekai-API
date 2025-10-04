package api

type Request struct {
	ApiType EndpointType
	Server  ServerRegion
	Path    string
	Query   map[string]string
}

func GetApiRequest(apiType EndpointType, server ServerRegion, subPath string, query map[string]string) *Request {
	req := &Request{
		ApiType: apiType,
		Server:  server,
		Path:    subPath,
		Query:   query,
	}
	return req
}
