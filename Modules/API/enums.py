from enum import Enum


class APIServerRegion(str, Enum):
    jp = "jp"
    en = "en"
    tw = "tw"
    kr = "kr"
    cn = "cn"


class APIType(str, Enum):
    api = "api"
    image = "image"
