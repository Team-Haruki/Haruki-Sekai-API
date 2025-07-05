import orjson
import asyncio
import aiofiles
from typing import Union
from pathlib import Path
from aiopath import AsyncPath
from aiohttp import ClientSession
from tenacity import retry, stop_after_attempt, wait_fixed


class SekaiCookieHelper:
    def __init__(self) -> None:
        self.cookies = None
        self.lock = asyncio.Lock()

    @retry(stop=stop_after_attempt(4), wait=wait_fixed(1))
    async def get_cookies(self, proxy: str = None) -> None:
        if not self.lock.locked():
            async with self.lock:
                headers = {
                    "Accept": "*/*",
                    "User-Agent": "ProductName/134 CFNetwork/1408.0.4 Darwin/22.5.0",
                    "Connection": "keep-alive",
                    "Accept-Language": "zh-CN,zh-Hans;q=0.9",
                    "Accept-Encoding": "gzip, deflate, br",
                    "X-Unity-Version": "2022.3.21f1",
                }
                async with ClientSession() as session:
                    async with session.post(
                        url="https://issue.sekai.colorfulpalette.org/api/signature", headers=headers, proxy=proxy
                    ) as response:
                        if response.status == 200:
                            self.cookies = response.headers.get("Set-Cookie")
                        else:
                            raise Exception()


class SekaiVersionHelper:
    def __init__(self, version_file_path: Union[AsyncPath, Path, str]) -> None:
        self.version_file_path = version_file_path
        self.app_version = None
        self.app_hash = None
        self.data_version = None
        self.asset_version = None
        self.lock = asyncio.Lock()

    async def get_app_version(self) -> None:
        if not self.lock.locked():
            async with self.lock:
                async with aiofiles.open(self.version_file_path, "r", encoding="utf-8") as f:
                    data = orjson.loads(await f.read())
                    self.app_version = data["appVersion"]
                    self.app_hash = data["appHash"]
                    self.data_version = data["dataVersion"]
                    self.asset_version = data["assetVersion"]
