from typing import Optional
from enum import Enum, IntEnum
from pydantic import BaseModel


class SekaiServerInfo(BaseModel):
    server: str
    api_url: str
    nuverse_master_data_url: Optional[str] = None
    require_cookies: Optional[bool] = False
    headers: Optional[dict] = None
    enabled: Optional[bool] = True
    aes_key: Optional[bytes] = None
    aes_iv: Optional[bytes] = None


class SekaiAccountCP(BaseModel):
    userId: int
    credential: str
    deviceId: Optional[str] = None


class SekaiAccountNuverse(BaseModel):
    accessToken: str
    userID: Optional[int] = 0
    deviceId: Optional[str] = None


class SekaiServerRegion(Enum):
    JP = 'jp'
    EN = 'en'
    TW = 'tw'
    KR = 'kr'
    CN = 'cn'


class SekaiApiHttpStatus(IntEnum):
    OK = 200
    CLIENT_ERROR = 400
    SESSION_ERROR = 403
    NOT_FOUND = 404
    CONFLICT = 409
    GAME_UPGRADE = 426
    SERVER_ERROR = 500
    UNDER_MAINTENANCE = 503
