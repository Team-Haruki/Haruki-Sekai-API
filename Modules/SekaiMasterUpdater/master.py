import orjson
import asyncio
import aiofiles
from pathlib import Path
from aiopath import AsyncPath
from typing import Dict, List, Union, Optional
from aiohttp import ClientSession, ClientResponse

from .compare import compare_version
from ..logger import AsyncLogger
from ..SekaiClient.manager import SekaiClientManager
from ..SekaiClient.model import SekaiServerRegion, SekaiServerInfo, SekaiApiHttpStatus, HarukiAssetUpdaterInfo

logger = AsyncLogger(__name__, level="DEBUG")


class SekaiMasterUpdater:
    def __init__(
        self,
        servers: Dict[SekaiServerRegion, SekaiServerInfo],
        managers: Dict[SekaiServerRegion, SekaiClientManager],
        master_dirs: Dict[SekaiServerRegion, Union[Path, str]],
        version_dirs: Dict[SekaiServerRegion, Union[Path, str]],
        asset_updater_servers: List[HarukiAssetUpdaterInfo],
    ) -> None:
        self.servers = servers
        self.managers = managers
        self.master_dirs = master_dirs
        self.version_dirs = version_dirs
        self.asset_updater_servers = asset_updater_servers

    @staticmethod
    async def load_file(file_path: Union[AsyncPath, Path, str]) -> Union[Dict, List]:
        async with aiofiles.open(file_path, "r", encoding="utf-8") as file:
            return orjson.loads(await file.read())

    @staticmethod
    async def save_file(file_path: Union[AsyncPath, Path, str], data: Union[Dict, List]) -> None:
        async with aiofiles.open(file_path, "wb") as file:
            await file.write(orjson.dumps(data, option=orjson.OPT_INDENT_2))

    # Call Haruki Sekai Asset Updater
    async def _call_asset_updater(self, options: Dict) -> Optional[ClientResponse]:
        async with ClientSession() as session:
            async with session.request(**options) as response:
                if response.status == SekaiApiHttpStatus.OK:
                    return response
                elif response.status == SekaiApiHttpStatus.CONFLICT:
                    await asyncio.sleep(60)
                    await self._call_asset_updater(options)
                    return None
                else:
                    return None

    async def asset_updater(self, server: SekaiServerRegion, data: Dict):
        body = {
            "server": server.value,
            "assetVersion": data.get("assetVersion"),
            "assetHash": data.get("assetHash", None),
        }
        for updater in self.asset_updater_servers:
            options = {
                "url": f"{updater.url}/update_asset",
                "method": "POST",
                "json": body,
                "headers": {"User-Agent": "Haruki Sekai API/v3.0.0"},
            }
            if updater.authorization:
                options["headers"]["Authorization"] = f"Bearer {updater.authorization}"
            asyncio.create_task(self._call_asset_updater(options))

    # Check update
    async def check_update(self, server: SekaiServerRegion, manager: SekaiClientManager) -> Optional[Dict]:
        _update_master = False
        _update_asset = False

        version_file = AsyncPath(self.version_dirs.get(server)) / "current_version.json"
        current_version = await self.load_file(version_file)
        current_server_version = await manager.get_login_data()
        current_data_version = current_version.get("dataVersion")
        current_asset_version = current_version.get("assetVersion")
        current_server_data_version = current_server_version.get("dataVersion")
        current_server_asset_version = current_server_version.get("assetVersion")
        current_server_asset_hash = current_server_version.get("assetHash", None)

        if server in [SekaiServerRegion.JP, SekaiServerRegion.EN]:
            if await asyncio.to_thread(compare_version, current_server_data_version, current_data_version):
                await logger.critical(
                    f"{server.value.upper()} server found new master data version: {current_server_data_version}"
                )
                _update_master = True
            if await asyncio.to_thread(compare_version, current_server_asset_version, current_asset_version):
                await logger.critical(
                    f"{server.value.upper()} server found new asset version: {current_server_data_version}"
                )
                _update_asset = True
        else:
            current_cdn_version = current_version.get("cdnVersion")
            current_server_cdn_version = current_server_version.get("cdnVersion")
            if int(current_cdn_version) < int(current_server_cdn_version):
                await logger.critical(
                    f"{server.value.upper()} server found new cdn version: {current_server_cdn_version}"
                )
                _update_master = True
                _update_asset = True

        if _update_asset:
            await self.asset_updater(server, current_server_version)
        if _update_master:
            await self.update_master(server, manager)

        if _update_asset or _update_master:
            current_version["dataVersion"] = current_server_data_version
            current_version["assetVersion"] = current_server_asset_version
            current_version["assetHash"] = current_server_asset_hash
            if current_server_version.get("cdnVersion"):
                current_version["cdnVersion"] = current_server_version.get("cdnVersion")
            await self.save_file(version_file, current_version)
            await self.save_file(
                AsyncPath(self.version_dirs.get(server)) / f"{current_server_data_version}.json", current_version
            )
            return {server: current_server_data_version}
        return None

    # Check update coroutine
    async def check_update_concurrently(self) -> Optional[Dict]:
        await logger.start()
        result_dict = {}
        tasks = [self.check_update(server, manager) for server, manager in self.managers.items()]
        results = await asyncio.gather(*tasks)
        result_dict.update(
            (key, value) for result in results if result and isinstance(result, dict) for key, value in result.items()
        )
        if result_dict:
            return result_dict
        await logger.stop()
        return None

    async def _save_split_master_data(self, server: SekaiServerRegion, master: Dict) -> None:
        await logger.info(
            f"Saving {server.value.upper()} server split master data...",
        )
        _master_dir = AsyncPath(self.master_dirs.get(server))
        tasks = [self.save_file(_master_dir / f"{key}.json", value) for key, value in master.items()]
        await asyncio.gather(*tasks)
        await logger.info(f"Saved {server.value.upper()} server split master data.")

    async def update_master(self, server: SekaiServerRegion, manager: SekaiClientManager) -> None:
        await logger.info(f"Downloading {server.value.upper()} new master data...")
        master_data = await manager.download_master()
        await logger.info(f"Downloaded {server.value.upper()} new master data.")
        await self._save_split_master_data(server, master_data)
