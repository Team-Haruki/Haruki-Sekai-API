import asyncio
import copy
import logging
import traceback
import coloredlogs
from uuid import uuid4
from copy import deepcopy
from urllib.parse import urlparse
from typing import Dict, Tuple, List, Optional, Union
from aiohttp import ClientSession, ClientResponse, ClientProxyConnectionError
from tenacity import retry, stop_after_attempt, wait_fixed, retry_if_not_exception_type

from Modules.log_format import LOG_FORMAT, FIELD_STYLE
from .cryptor import SekaiCryptor
from .nuverse import nuverse_master_restorer
from .helper import SekaiCookieHelper, SekaiVersionHelper
from .exceptions import SessionError, UpgradeRequiredError, UnderMaintenanceError, UnknownSekaiClientException, \
    CookieExpiredError
from .model import SekaiServerInfo, SekaiAccountCP, SekaiAccountNuverse, SekaiServerRegion, SekaiApiHttpStatus

logger = logging.getLogger(__name__)
coloredlogs.install(level='DEBUG', logger=logger, fmt=LOG_FORMAT, field_styles=FIELD_STYLE)


class SekaiClient:
    def __init__(self, server_info: SekaiServerInfo, account: Union[SekaiAccountCP, SekaiAccountNuverse],
                 key: bytes, iv: bytes, cookie_helper: Optional[SekaiCookieHelper] = None,
                 version_helper: Optional[SekaiVersionHelper] = None,
                 proxies: Optional[Union[List, str]] = None) -> None:
        # Server configs
        self.server = SekaiServerRegion(server_info.server)
        self.api_url = server_info.api_url
        self.nuverse_master_data_url = server_info.nuverse_master_data_url
        self.require_cookies = server_info.require_cookies
        self.headers = deepcopy(server_info.headers)
        self.cryptor = SekaiCryptor(key, iv)
        # Account configs
        self.account = account
        self.user_id = account.userId if isinstance(account, SekaiAccountCP) else account.userID
        # Helper configs
        self.cookie_helper = cookie_helper
        self.version_helper = version_helper
        # Other configs
        self.proxies = proxies if proxies is not None else ['']
        self.lock = asyncio.Lock()
        self.session: Optional[ClientSession] = None

    # (Japanese server only) Parse CloudFront cookies
    async def parse_cookies(self) -> None:
        if self.server != SekaiServerRegion.JP:
            return None
        else:
            await self.cookie_helper.get_cookies()
            await asyncio.sleep(1)
            self.headers["Cookie"] = self.cookie_helper.cookies

    # Parse version data
    async def parse_version(self) -> None:
        await self.version_helper.get_app_version()
        await asyncio.sleep(1)
        self.headers["appVersion"] = self.version_helper.app_version
        self.headers["appHash"] = self.version_helper.app_hash
        self.headers["dataVersion"] = self.version_helper.data_version
        self.headers["assetVersion"] = self.version_helper.asset_version

    # Init client
    async def init(self) -> None:
        await self.parse_cookies()
        await self.parse_version()
        self.session = ClientSession()

    # Sekai API response handler
    async def _response(self, response: ClientResponse) -> Optional[Tuple[Dict, int]]:
        if response.content_type in ['application/octet-stream', 'binary/octet-stream']:
            _unpack_response = await self.cryptor.unpack(await response.read())
            if response.status in [SekaiApiHttpStatus.OK, SekaiApiHttpStatus.CLIENT_ERROR,
                                   SekaiApiHttpStatus.NOT_FOUND, SekaiApiHttpStatus.CONFLICT]:
                return _unpack_response, response.status
            elif response.status == SekaiApiHttpStatus.SESSION_ERROR:
                raise SessionError
            elif response.status == SekaiApiHttpStatus.GAME_UPGRADE:
                raise UpgradeRequiredError
            elif response.status == SekaiApiHttpStatus.UNDER_MAINTENANCE:
                raise UnderMaintenanceError
        else:
            if response.status == SekaiApiHttpStatus.SERVER_ERROR:
                raise UnknownSekaiClientException(response.status, await response.read())
            if response.status == SekaiApiHttpStatus.SESSION_ERROR and response.content_type == 'text/xml':  # Japanese Server Only
                raise CookieExpiredError

    # Call sekai server API
    @retry(
        stop=stop_after_attempt(4),
        wait=wait_fixed(1),
        retry=retry_if_not_exception_type((UpgradeRequiredError, CookieExpiredError, UnderMaintenanceError))
    )
    async def call_api(self, path: str, method: str = 'GET', data: Optional[Union[bytes, Dict, List]] = None,
                       params: Optional[Dict] = None) -> Optional[Tuple[Dict, int]]:
        # Reserved exception variable
        last_exception = None

        # Init request args
        self.headers['X-Request-Id'] = str(uuid4())
        if path.startswith('/user/%user_id'):
            sub_path = path.replace('/user/%user_id', '')
            new_path = f'/api/user/{self.user_id}{sub_path}'
        else:
            new_path = f'/api{path}'
        logger.info(f'{self.server.value.upper()} server account #{self.user_id} {method} {new_path}')
        url = f"{self.api_url}{new_path}"
        options = {
            'url': url,
            'method': method,
            'headers': self.headers,
            'params': params,
            'data': await self.cryptor.pack(data) if data is not None else None,
        }

        # Try to request Game API
        for proxy in self.proxies:
            try:
                async with self.session.request(**options, proxy=proxy) as response:
                    if 'X-Session-Token' in response.headers:
                        self.headers['X-Session-Token'] = response.headers['X-Session-Token']
                    return await self._response(response)
            except ClientProxyConnectionError as e:
                logger.warning(f"Failed to connect proxy {proxy}, switching proxy and retrying...")
                last_exception = e
                continue
            except SessionError:
                logger.warning(
                    f'{self.server.value.upper()} server client #{self.user_id} session expired, re-logging in...')
                await self.login()
                raise SessionError
            except CookieExpiredError:
                logger.warning('JP server clients\' cookies expired, re-parsing cookies...')
                await self.parse_cookies()
                raise CookieExpiredError
            except asyncio.TimeoutError:
                logger.warning(
                    f'{self.server.value.upper()} server client #{self.user_id} request timed out, retrying...')
                raise asyncio.TimeoutError
            except UpgradeRequiredError:
                logger.warning(f'{self.server.value.upper()} server app version might be upgraded')
                raise UpgradeRequiredError
            except UnderMaintenanceError:
                logger.warning(f'{self.server.value.upper()} server is under maintenance')
                raise UnderMaintenanceError
            except Exception as e:
                traceback.print_exc()
                logger.error(
                    f"An error occurred: server = {self.server.value.upper()}, exception = {repr(e)}")

        logger.warning("Failed to use all proxies, retrying...")
        if last_exception:
            raise last_exception

    # (Japanese Serve Only) Get MySekai image
    async def get_image(self, path: str) -> Optional[Tuple[Union[bytes, str], int]]:
        url = f'{self.api_url}/image{path}'
        for proxy in self.proxies:
            try:
                async with self.session.get(url, proxy=proxy, headers=self.headers) as response:
                    if response.status == SekaiApiHttpStatus.OK:
                        return await response.read(), response.status
                    else:
                        return 'Error', response.status
            except ClientProxyConnectionError:
                logger.warning(f"Failed to connect proxy {proxy}, switching proxy and retrying...")
                continue
        else:
            return 'Error', SekaiApiHttpStatus.SERVER_ERROR

    # Login game
    async def login(self) -> Optional[Dict]:
        # Pack body
        data = await self.cryptor.pack(self.account.model_dump())

        # Generate Login URL
        if isinstance(self.account, SekaiAccountCP):
            url = f"{self.api_url}/api/user/{self.user_id}/auth?refreshUpdatedResources=False"
        else:
            url = f"{self.api_url}/api/user/auth"

        # Request options
        options = {
            'url': url,
            'method': 'PUT' if isinstance(self.account, SekaiAccountCP) else 'POST',
            'headers': self.headers,
            'data': data,
            'timeout': 20
        }

        # Try to login
        for proxy in self.proxies:
            try:
                async with self.session.request(**options, proxy=proxy) as response:
                    if response.status == SekaiApiHttpStatus.GAME_UPGRADE:
                        logger.warning(f'{self.server.value.upper()} server app version might be upgraded')
                        raise UpgradeRequiredError
                    elif response.status == SekaiApiHttpStatus.OK:
                        data = await self.cryptor.unpack(await response.read())
                        self.headers['X-Session-Token'] = data.get('sessionToken')
                        self.headers['X-Data-Version'] = data.get('dataVersion')
                        self.headers['X-Asset-Version'] = data.get('assetVersion')
                        if isinstance(self.account, SekaiAccountNuverse):
                            self.user_id = data.get('userRegistration').get('userId')
                        logger.info(f'{self.server.value.upper()} server account #{self.user_id} logged in.')
                        return data
                    else:
                        logger.warning(
                            f'{self.server.value} account login failed with status {response.status},'
                            f' {await self.cryptor.unpack(await response.read())}'
                        )
            except UpgradeRequiredError:
                raise UpgradeRequiredError
            except Exception as e:
                logger.error(f'An error occurred: server = {self.server.value.upper()}, exception = {repr(e)}')
                raise e

    # Download master data
    async def download_master(self) -> Dict:
        master_data = {}
        login_data = await self.login()
        if isinstance(self.account, SekaiAccountNuverse):
            cdn_version = login_data.get('cdnVersion')
            headers = copy.deepcopy(self.headers)
            url = f'{self.nuverse_master_data_url}/master-data-{cdn_version}.info'
            hostname = urlparse(url).hostname
            headers['Host'] = hostname
            for proxy in self.proxies:
                try:
                    async with self.session.get(url, headers=headers, proxy=proxy) as response:
                        data = await self.cryptor.unpack(await response.read())
                        return await asyncio.to_thread(nuverse_master_restorer, data)
                except ClientProxyConnectionError:
                    logger.warning(f"Failed to connect proxy {proxy}, switching proxy and retrying...")
        else:
            _split_master = login_data.get('suiteMasterSplitPath')
            for _path in _split_master:
                result = await self.call_api(f'/{_path}')
                data, _ = result
                master_data.update(data)

        return master_data

    # API GET
    async def get(self, path: str, params: Optional[Dict] = None) -> Optional[Tuple[Dict, int]]:
        try:
            return await self.call_api(path, params=params)
        except Exception as e:
            logger.error(f'An error occurred: {repr(e)}')
            raise e

    # Shutdown client
    async def close(self) -> None:
        await self.session.close()
