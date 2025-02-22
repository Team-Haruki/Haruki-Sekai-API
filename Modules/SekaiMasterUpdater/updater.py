import asyncio
import logging
import aiofiles
import coloredlogs
import ujson as json
from pathlib import Path
from typing import Dict, List, Union, Optional
from aiohttp import ClientSession, ClientResponse

from ..SekaiClient.model import SekaiServerRegion, SekaiServerInfo
from ..SekaiClient.manager import SekaiClientManager

from Modules.log_format import LOG_FORMAT, FIELD_STYLE

logger = logging.getLogger(__name__)
coloredlogs.install(level='DEBUG', logger=logger, fmt=LOG_FORMAT, field_styles=FIELD_STYLE)


class SekaiMasterUpdater:
    def __init__(self, servers: Dict[SekaiServerRegion, SekaiServerInfo],
                 managers: Dict[SekaiServerRegion, SekaiClientManager],
                 master_dirs: Dict[SekaiServerRegion, Union[Path, str]],
                 version_dirs: Dict[SekaiServerRegion, Union[Path, str]], asset_updater_url: str) -> None:
        self.servers = servers
        self.managers = managers
        self.master_dirs = master_dirs
        self.version_dirs = version_dirs
        self.asset_updater_url = asset_updater_url

    @staticmethod
    async def compare_version(new_version, current_version) -> bool:
        _new_version_parts = new_version.split('.')
        _current_version_parts = current_version.split('.')

        for i in range(len(_new_version_parts)):
            if int(_new_version_parts[i]) > int(_current_version_parts[i]):
                return True
            elif int(_new_version_parts[i]) < int(_current_version_parts[i]):
                return False

        return False

    @staticmethod
    async def load_file(file_path) -> Union[Dict, List]:
        async with aiofiles.open(file_path, 'r', encoding='utf-8') as file:
            return json.loads(await file.read())

    @staticmethod
    async def save_file(file_path, data) -> None:
        async with aiofiles.open(file_path, 'w', encoding='utf-8') as file:
            await file.write(json.dumps(data, indent=4, ensure_ascii=False))

    # Call Haruki Sekai Asset Updater
    async def call_asset_updater(self, server: SekaiServerRegion, data: Dict) -> Optional[ClientResponse]:
        body = {
            'server': server.value,
            'assetVersion': data.get('assetVersion'),
            'assetHash': data.get('assetHash', None)
        }
        options = {
            'url': f'{self.asset_updater_url}/update_asset',
            'method': 'POST',
            'data': body,
            'headers': {'User-Agent': 'Haruki Sekai API/v2.0.0'}
        }
        async with ClientSession() as session:
            async with session.request(**options) as response:
                return response

    # Check update
    async def check_update(self, server: SekaiServerRegion, manager: SekaiClientManager) -> Optional[Dict]:
        _update_master = False
        _update_asset = False

        version_file = Path(self.version_dirs.get(server)) / 'current_version.json'
        current_version = await self.load_file(version_file)
        current_data_version = current_version.get('dataVersion')
        current_asset_version = current_version.get('assetVersion')
        current_server_version = await manager.get_login_data()
        current_server_data_version = current_server_version.get('dataVersion')
        current_server_asset_version = current_server_version.get('assetVersion')
        current_server_asset_hash = current_server_version.get('assetHash', None)
        if await self.compare_version(current_server_data_version, current_data_version):
            logger.critical(
                f'{server.value.upper()} server found new master data version: {current_server_data_version}')
            _update_master = True
        if await self.compare_version(current_server_asset_version, current_asset_version):
            logger.critical(
                f'{server.value.upper()} server found new asset version: {current_server_data_version}')
            _update_asset = True

        if _update_asset:
            # await self.call_asset_updater(server, current_server_version)
            pass
        if _update_master:
            await self.update_master(server, manager)
        if _update_asset or _update_master:
            current_version['dataVersion'] = current_server_data_version
            current_version['assetVersion'] = current_server_asset_version
            current_version['assetHash'] = current_server_asset_hash
            if current_server_version.get('cdnVersion'):
                current_version['cdnVersion'] = current_server_version.get('cdnVersion')
            await self.save_file(version_file, current_version)
            await self.save_file(Path(self.version_dirs.get(server)) / f'{current_server_data_version}.json',
                                 current_version)
            return {server: current_server_data_version}

    # Check update coroutine
    async def check_update_coroutine(self) -> Optional[Dict]:
        result_dict = {}
        tasks = [self.check_update(server, manager) for server, manager in self.managers.items()]
        results = await asyncio.gather(*tasks)
        result_dict.update(
            (key, value) for result in results if result and isinstance(result, dict) for key, value in result.items()
        )
        if result_dict:
            return result_dict

    async def _save_split_master_data(self, server: SekaiServerRegion, master: Dict) -> None:
        logger.info(f'Saving {server.value.upper()} server split master data...', )
        _master_dir = Path(self.master_dirs.get(server))
        tasks = [self.save_file(_master_dir / f'{key}.json', value) for key, value in master.items()]
        await asyncio.gather(*tasks)
        logger.info(f'Saved {server.value.upper()} server split master data.')

    async def update_master(self, server: SekaiServerRegion, manager: SekaiClientManager) -> None:
        logger.info(f'Downloading {server.value.upper()} new master data...')
        master_data = await manager.download_master()
        logger.info(f'Downloaded {server.value.upper()} new master data.')
        await self._save_split_master_data(server, master_data)
