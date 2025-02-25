import asyncio
import logging
import aiofiles
import coloredlogs
import ujson as json
from pathlib import Path
from typing import Dict, Tuple, List, Union, Optional

from Modules.log_format import LOG_FORMAT, FIELD_STYLE
from .client import SekaiClient
from .helper import SekaiCookieHelper, SekaiVersionHelper
from .exceptions import UpgradeRequiredError, UnderMaintenanceError, CookieExpiredError
from .model import SekaiServerInfo, SekaiAccountCP, SekaiAccountNuverse, SekaiServerRegion

logger = logging.getLogger(__name__)
coloredlogs.install(level='DEBUG', logger=logger, fmt=LOG_FORMAT, field_styles=FIELD_STYLE)


class SekaiClientManager:
    def __init__(self, server_info: SekaiServerInfo, accounts_dir: Union[Path, str],
                 version_file_path: Union[Path, str], proxies: Optional[List] = None) -> None:
        # Server configs
        self.server: SekaiServerRegion = SekaiServerRegion(server_info.server)
        self.server_info: SekaiServerInfo = server_info
        self.version_helper: SekaiVersionHelper = SekaiVersionHelper(version_file_path)
        self.cookie_helper: Union[
            SekaiCookieHelper, None] = SekaiCookieHelper() if self.server == SekaiServerRegion.JP else None
        # Account configs
        self.accounts_dir: Union[Path, str] = accounts_dir
        self.clients: List[SekaiClient] = []
        # Proxies config
        self.proxies = proxies

    # Generate an account pool
    async def _parse_accounts(self) -> List[Union[SekaiAccountCP, SekaiAccountNuverse]]:
        accounts = []
        for json_file in Path(self.accounts_dir).rglob('*.json'):
            async with aiofiles.open(json_file, mode='r', encoding='utf-8') as f:
                try:
                    data = json.loads(await f.read())
                except json.JSONDecodeError as e:
                    logger.warning(f"Error decoding JSON in file {json_file}: {e}")
                    continue
                if isinstance(data, dict):
                    if self.server in [SekaiServerRegion.JP, SekaiServerRegion.EN]:
                        account = SekaiAccountCP(**data)
                    else:
                        account = SekaiAccountNuverse(**data)
                    accounts.append(account)
                elif isinstance(data, list):
                    for _account in data:
                        if self.server in [SekaiServerRegion.JP, SekaiServerRegion.EN]:
                            account = SekaiAccountCP(**_account)
                        else:
                            account = SekaiAccountNuverse(**_account)
                        accounts.append(account)
                else:
                    logger.warning(f"Unexpected data type in file {json_file}: {type(data)}")
                    continue

        return accounts

    # Re-parse cookies
    async def _parse_cookies(self) -> None:
        if self.server == SekaiServerRegion.JP:
            tasks = [client.parse_cookies() for client in self.clients]
            await asyncio.gather(*tasks)

    # Re-parse version data
    async def _parse_version(self) -> None:
        tasks = [client.parse_version() for client in self.clients]
        await asyncio.gather(*tasks)

    # Init manager
    async def init(self) -> None:
        # Create clients list
        _accounts = await  self._parse_accounts()
        self.clients.extend([
            SekaiClient(self.server_info, _account, self.server_info.aes_key, self.server_info.aes_iv,
                        self.cookie_helper, self.version_helper, self.proxies) for _account in _accounts
        ])
        # Init clients
        client_init_tasks = [client.init() for client in self.clients]
        await asyncio.gather(*client_init_tasks)

        # Login
        login_tasks = [client.login() for client in self.clients]
        await asyncio.gather(*login_tasks)

    # Get login data
    async def get_login_data(self) -> Optional[Dict]:
        for client in self.clients:
            if not client.lock.locked():
                async with client.lock:
                    return await client.login()
            else:
                continue

    # Download master data
    async def download_master(self) -> Optional[Dict]:
        for client in self.clients:
            if not client.lock.locked():
                async with client.lock:
                    return await client.download_master()

    # Call game API
    async def api_get(self, path: str, params: Optional[Dict] = None) -> Optional[Tuple[Dict, int]]:
        for client in self.clients:
            if not client.lock.locked():
                async with client.lock:
                    try:
                        return await client.get(path, params=params)
                    except CookieExpiredError:
                        logger.warning(f"{self.server.value.upper()} Server cookies expired, re-parsing...")
                        await self._parse_cookies()
                        continue
                    except UpgradeRequiredError:
                        logger.warning(f"{self.server.value.upper()} Server upgrade required, re-parsing...")
                        await self._parse_version()
                        continue
                    except UnderMaintenanceError:
                        logger.warning(f"{self.server.value.upper()} Server is under maintenance.")
                        error_response = {
                            'result': 'failed',
                            'message': 'JP Game server is under maintenance.'
                        }
                        return error_response, 503
                    except Exception as e:
                        logger.warning(f"Failed to call {self.server.value.upper()} Server API: {repr(e)}")
                        error_response = {
                            'result': 'failed',
                            'message': repr(e)
                        }
                        return error_response, 500
        else:
            error_response = {
                'result': 'failed',
                'message': 'No client is available, please try again later.'
            }
            return error_response, 500

    async def image_get(self, path: str) -> Optional[Tuple[Union[bytes, str], int]]:
        for client in self.clients:
            if not client.lock.locked():
                return await client.get_image(path)

    async def shutdown(self) -> None:
        tasks = [client.close() for client in self.clients]
        await asyncio.gather(*tasks)
