import asyncio
from pathlib import Path

from Modules.SekaiClient.manager import SekaiClientManager
from Modules.SekaiMasterUpdater.updater import SekaiMasterUpdater
from Modules.SekaiMasterUpdater.git import GitUpdater

from configs import (SEKAI_SERVERS, ACCOUNTS_DIRS, VERSION_SAVE_DIRS, MASTER_SAVE_DIRS, ASSET_UPDATER_URL,
                     ENABLE_GIT_PUSH, GIT_USER, GIT_EMAIL, GIT_PASS, REPOS, PROXIES)

_servers = {server: server_info for server, server_info in SEKAI_SERVERS.items() if server_info.enabled}
managers = {
    server: SekaiClientManager(server_info, Path(ACCOUNTS_DIRS.get(server)),
                               Path(VERSION_SAVE_DIRS.get(server)) / 'current_version.json')
    for server, server_info in _servers.items()
}

updater = SekaiMasterUpdater(_servers, managers, MASTER_SAVE_DIRS, VERSION_SAVE_DIRS, ASSET_UPDATER_URL)

if ENABLE_GIT_PUSH:
    git_updater = GitUpdater(GIT_USER, GIT_EMAIL, GIT_PASS, PROXIES)
else:
    git_updater = None


async def check_update() -> None:
    result = await updater.check_update_coroutine()
    if git_updater:
        if result:
            tasks = [asyncio.to_thread(git_updater.push_remote, REPOS.get(server), data_version) for
                     server, data_version in result.items()]
            await asyncio.gather(*tasks)
