import jwt
import asyncio
from pathlib import Path
from sqlalchemy import select
from redis.asyncio import Redis
from fastapi import Request, Depends, HTTPException
from apscheduler.schedulers.asyncio import AsyncIOScheduler
from sqlalchemy.ext.asyncio import AsyncSession, create_async_engine, async_sessionmaker

from Modules.SekaiMasterUpdater.git import GitUpdater
from Modules.API.tables import SekaiUser, SekaiUserServer
from Modules.API.utils import APIRequest, get_api_request
from Modules.SekaiClient.manager import SekaiClientManager
from Modules.SekaiMasterUpdater.apphash import AppHashUpdater
from Modules.SekaiMasterUpdater.master import SekaiMasterUpdater
from configs import (
    REPOS,
    PROXIES,
    GIT_USER,
    GIT_PASS,
    GIT_EMAIL,
    REDIS_HOST,
    REDIS_PORT,
    JWT_SECRET,
    DATABASE_URL,
    SEKAI_SERVERS,
    ACCOUNTS_DIRS,
    REDIS_PASSWORD,
    ENABLE_GIT_PUSH,
    MASTER_SAVE_DIRS,
    VERSION_SAVE_DIRS,
    ASSET_UPDATER_SERVERS,
    SEKAI_APPHASH_SOURCES,
)


scheduler = AsyncIOScheduler()
engine = create_async_engine(DATABASE_URL)
async_session = async_sessionmaker(engine, expire_on_commit=False)
redis_client = Redis(host=REDIS_HOST, port=REDIS_PORT, password=REDIS_PASSWORD, decode_responses=True)
_servers = {server: server_info for server, server_info in SEKAI_SERVERS.items() if server_info.enabled}
managers = {
    server: SekaiClientManager(
        server_info,
        Path(ACCOUNTS_DIRS.get(server)),
        Path(VERSION_SAVE_DIRS.get(server)) / "current_version.json",
        PROXIES,
    )
    for server, server_info in _servers.items()
}
master_updater = SekaiMasterUpdater(_servers, managers, MASTER_SAVE_DIRS, VERSION_SAVE_DIRS, ASSET_UPDATER_SERVERS)

if ENABLE_GIT_PUSH:
    git_updater = GitUpdater(GIT_USER, GIT_EMAIL, GIT_PASS, PROXIES)
else:
    git_updater = None


async def check_master_update() -> None:
    result = await master_updater.check_update_concurrently()
    if git_updater:
        if result:
            tasks = [
                asyncio.to_thread(git_updater.push_remote, REPOS.get(server), data_version)
                for server, data_version in result.items()
            ]
            await asyncio.gather(*tasks)


async def check_app_update() -> None:
    app_updater = AppHashUpdater(SEKAI_APPHASH_SOURCES, VERSION_SAVE_DIRS)
    await app_updater.init()
    await app_updater.check_app_version_concurrently()
    await app_updater.shutdown()


def validate_user_token(
    _async_session: async_sessionmaker[AsyncSession],
):
    async def _validate_user_token(
        request: Request,
        api_request: APIRequest = Depends(get_api_request),
    ) -> SekaiUser:
        token = request.headers.get("X-Haruki-Sekai-Token")
        if not token:
            raise HTTPException(status_code=401, detail="Missing token")

        try:
            payload = jwt.decode(token, JWT_SECRET, algorithms=["HS256"])
            user_id = payload.get("uid")
            credential = payload.get("credential")
            if not user_id or not credential:
                raise ValueError("Invalid token payload")
        except Exception:
            raise HTTPException(status_code=401, detail="Invalid token")

        redis_key = f"haruki_sekai_api:{user_id}:{api_request.server}"
        cached = await redis_client.get(redis_key)
        if cached:
            return SekaiUser(id=user_id, credential=credential, remark="")

        async with _async_session() as session:
            result = await session.execute(
                select(SekaiUserServer)
                .where(SekaiUserServer.user_id == user_id)
                .where(SekaiUserServer.server == api_request.server)
            )
            if not result.scalar():
                raise HTTPException(status_code=403, detail="Not authorized for this server")

        await redis_client.set(redis_key, "1", ex=43200)
        return SekaiUser(id=user_id, credential=credential, remark="")

    return _validate_user_token
