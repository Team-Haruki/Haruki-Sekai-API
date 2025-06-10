from fastapi import Request
from pydantic import BaseModel
from typing import Optional, Dict
from Modules.API.enums import APIType, APIServerRegion


class APIRequest(BaseModel):
    api_type: APIType
    server: APIServerRegion
    path: str
    query_params: Optional[Dict] = None


async def get_api_request(api_type: APIType, server: APIServerRegion, sub_path: str, request: Request) -> APIRequest:
    return APIRequest(
        api_type=api_type, server=server, path="/" + sub_path, query_params=dict(request.query_params) or None
    )
