import orjson
import asyncio
from pathlib import Path
from aiopath import AsyncPath
from aiohttp import ClientSession
from pydantic import ValidationError
from typing import Optional, Dict, List, Union

from .compare import compare_version
from ..logger import AsyncLogger
from ..SekaiClient.model import SekaiServerRegion, HarukiAppHashSource, HarukiAppHashSourceType, HarukiAppInfo

logger = AsyncLogger(__name__, level="DEBUG")


class AppHashUpdater(object):
    def __init__(
        self,
        sources: List[HarukiAppHashSource],
        server_version_dirs: Dict[SekaiServerRegion, Union[AsyncPath, Path, str]],
    ) -> None:
        self._sources = sources
        self._server_version_dirs = server_version_dirs
        self._session: Optional[ClientSession] = None

    async def init(self) -> None:
        self._session = ClientSession()
        await logger.start()

    async def get_remote_app_version(
        self, server: SekaiServerRegion, source: HarukiAppHashSource
    ) -> Optional[HarukiAppInfo]:
        filename = f"{server.value.upper()}.json"
        match source.type_:
            case HarukiAppHashSourceType.FILE:
                path = AsyncPath(source.dir) / filename
                if await path.exists():
                    try:
                        data = await path.read_text(encoding="utf-8")
                        return await asyncio.to_thread(lambda: HarukiAppInfo(**orjson.loads(data)))
                    except (orjson.JSONDecodeError, ValidationError):
                        return None
                return None
            case HarukiAppHashSourceType.URL:
                url = source.url + filename
                async with self._session.get(url) as response:
                    if response.status == 200:
                        try:
                            data = await response.json()
                            return await asyncio.to_thread(lambda: HarukiAppInfo(**data))
                        except ValidationError:
                            return None
                    return None
        return None

    async def get_latest_remote_app_info(self, server: SekaiServerRegion) -> Optional[HarukiAppInfo]:
        results = await asyncio.gather(*[self.get_remote_app_version(server, source) for source in self._sources])
        valid_results = [r for r in results if r is not None]
        if not valid_results:
            return None

        def version_key(app: HarukiAppInfo):
            return [int(p) for p in app.appVersion.split(".")]

        return max(valid_results, key=version_key)

    async def get_current_app_version(self, server: SekaiServerRegion) -> Optional[HarukiAppInfo]:
        path = AsyncPath(self._server_version_dirs.get(server)) / "current_version.json"
        try:
            data = await path.read_text(encoding="utf-8")
            return await asyncio.to_thread(lambda: HarukiAppInfo(**orjson.loads(data)))
        except (OSError, orjson.JSONDecodeError, ValidationError):
            return None

    async def save_new_app_hash(self, server: SekaiServerRegion, app: HarukiAppInfo) -> bool:
        try:
            path = AsyncPath(self._server_version_dirs.get(server)) / "current_version.json"
            try:
                raw = await path.read_text(encoding="utf-8")
                data = await asyncio.to_thread(orjson.loads, raw)
            except Exception:
                data = {}
            data["appVersion"] = app.appVersion
            data["appHash"] = app.appHash
            json_bytes = await asyncio.to_thread(orjson.dumps, data, option=orjson.OPT_INDENT_2)
            await path.write_bytes(json_bytes)
            return True
        except Exception:
            return False

    async def check_app_version(self, server: SekaiServerRegion) -> bool:
        local_version = await self.get_current_app_version(server)
        remote_version = await self.get_latest_remote_app_info(server)
        if not local_version or not remote_version:
            await logger.warning(f"{server.value.upper()} server: local or remote version is unavailable.")
            return False
        is_newer = await asyncio.to_thread(compare_version, remote_version.appVersion, local_version.appVersion)
        if is_newer:
            await logger.info(
                f"{server.value.upper()} server found new app version: {remote_version.appVersion},"
                f" saving new app hash..."
            )
            save_result = await self.save_new_app_hash(server, remote_version)
            if save_result:
                await logger.info(f"{server.value.upper()} server saved new app hash.")
            else:
                await logger.warning(f"{server.value.upper()} server failed to save new app hash.")
            return save_result
        await logger.info(f"{server.value.upper()} server no new app version found.")
        return False

    async def check_app_version_concurrently(self) -> None:
        tasks = [self.check_app_version(server) for server in self._server_version_dirs]
        await asyncio.gather(*tasks)

    async def shutdown(self) -> None:
        if self._session:
            await self._session.close()
        await logger.stop()
