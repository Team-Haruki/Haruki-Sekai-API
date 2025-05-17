import asyncio
import aiofiles
import ujson as json
from pathlib import Path
from typing import Dict, Tuple, List, Union, Optional
from tenacity import retry, stop_after_attempt, wait_fixed, retry_if_exception_type

from ..logger import AsyncLogger
from .client import SekaiClient
from .helper import SekaiCookieHelper, SekaiVersionHelper
from .exceptions import UpgradeRequiredError, UnderMaintenanceError, CookieExpiredError
from .model import SekaiServerInfo, SekaiAccountCP, SekaiAccountNuverse, SekaiServerRegion

logger = AsyncLogger(__name__, level="DEBUG")


class SekaiClientManager:
    def __init__(
        self,
        server_info: SekaiServerInfo,
        accounts_dir: Union[Path, str],
        version_file_path: Union[Path, str],
        proxies: Optional[List] = None,
    ) -> None:
        # Server configs
        self.server: SekaiServerRegion = SekaiServerRegion(server_info.server)
        self.server_info: SekaiServerInfo = server_info
        self.version_helper: SekaiVersionHelper = SekaiVersionHelper(version_file_path)
        self.cookie_helper: Union[SekaiCookieHelper, None] = (
            SekaiCookieHelper() if self.server == SekaiServerRegion.JP else None
        )
        # Account configs
        self.accounts_dir: Union[Path, str] = accounts_dir
        self.clients: List[SekaiClient] = []
        self.client_no: int = 0
        # Proxies config
        self.proxies = proxies

    # Generate an account pool
    async def _parse_accounts(self) -> List[Union[SekaiAccountCP, SekaiAccountNuverse]]:
        accounts = []
        for json_file in Path(self.accounts_dir).rglob("*.json"):
            async with aiofiles.open(json_file, mode="r", encoding="utf-8") as f:
                try:
                    data = json.loads(await f.read())
                except json.JSONDecodeError as e:
                    await logger.warning(f"Error decoding JSON in file {json_file}: {e}")
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
                    await logger.warning(f"Unexpected data type in file {json_file}: {type(data)}")
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
        await logger.start()
        # Create clients list
        _accounts = await self._parse_accounts()
        self.clients.extend(
            [
                SekaiClient(
                    self.server_info,
                    _account,
                    self.server_info.aes_key,
                    self.server_info.aes_iv,
                    self.cookie_helper,
                    self.version_helper,
                    self.proxies,
                )
                for _account in _accounts
            ]
        )
        # Init clients
        client_init_tasks = [client.init() for client in self.clients]
        await asyncio.gather(*client_init_tasks)

        # Login
        try:
            login_tasks = [client.login() for client in self.clients]
            await asyncio.gather(*login_tasks)
        except Exception as e:
            await logger.error(f"Error while initializing Sekai client: {e}")

    # Get a client
    async def get_client(self) -> Optional[SekaiClient]:
        if self.client_no == len(self.clients):
            self.client_no = 0
            return self.clients[self.client_no]
        else:
            self.client_no += 1
            return self.clients[self.client_no - 1]

    # Get login data
    async def get_login_data(self) -> Optional[Dict]:
        client = await self.get_client()
        if not client.lock.locked():
            async with client.lock:
                return await client.login()
        return None

    # Download master data
    async def download_master(self) -> Optional[Dict]:
        client = await self.get_client()
        if not client.lock.locked():
            async with client.lock:
                return await client.download_master()
        return None

    # Call game API
    @retry(
        stop=stop_after_attempt(4),
        wait=wait_fixed(1),
        retry=retry_if_exception_type((UpgradeRequiredError, CookieExpiredError)),
    )
    async def api_get(self, path: str, params: Optional[Dict] = None) -> Optional[Tuple[Dict, int]]:
        client = await self.get_client()
        if not client.lock.locked():
            async with client.lock:
                try:
                    return await client.get(path, params=params)
                except CookieExpiredError:
                    await logger.warning(f"{self.server.value.upper()} Server cookies expired, re-parsing...")
                    await self._parse_cookies()
                except UpgradeRequiredError:
                    await logger.warning(f"{self.server.value.upper()} Server upgrade required, re-parsing...")
                    await self._parse_version()
                except UnderMaintenanceError:
                    await logger.warning(f"{self.server.value.upper()} Server is under maintenance.")
                    error_response = {
                        "result": "failed",
                        "message": f"{self.server.value.upper()} Game server is under maintenance.",
                    }
                    return error_response, 503
                except Exception as e:
                    await logger.warning(f"Failed to call {self.server.value.upper()} Server API: {repr(e)}")
                    error_response = {"result": "failed", "message": repr(e)}
                    return error_response, 500
        else:
            error_response = {"result": "failed", "message": "No client is available, please try again later."}
            return error_response, 500

    async def image_get(self, path: str) -> Optional[Tuple[Union[bytes, str], int]]:
        client = await self.get_client()
        if not client.lock.locked():
            return await client.get_image(path)
        return None

    async def shutdown(self) -> None:
        tasks = [client.close() for client in self.clients]
        await asyncio.gather(*tasks)
        await logger.stop()
