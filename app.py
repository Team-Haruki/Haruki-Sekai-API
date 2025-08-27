import asyncio
import traceback
from typing import Union
from contextlib import asynccontextmanager
from fastapi.responses import ORJSONResponse
from apscheduler.triggers.cron import CronTrigger
from fastapi import FastAPI, HTTPException, Depends
from starlette.responses import Response as StarletteResponse

from Modules.API.tables import Base
from Modules.API.enums import APIType
from Modules.SekaiClient.model import SekaiServerRegion
from Modules.API.utils import APIRequest, get_api_request
from utils import check_master_update, check_app_update, scheduler, managers, engine, async_session, validate_user_token

from nuverse import nvapi

@asynccontextmanager
async def lifespan(_app: FastAPI):
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
    tasks = [manager.init() for _, manager in managers.items()]
    await asyncio.gather(*tasks)
    scheduler.add_job(check_master_update, CronTrigger(second=7))
    scheduler.add_job(check_app_update, CronTrigger(second=30, minute="*/5"))
    scheduler.start()
    yield
    scheduler.shutdown()
    tasks = [manager.shutdown() for _, manager in managers.items()]
    await asyncio.gather(*tasks)
    await engine.dispose()


app = FastAPI(lifespan=lifespan, default_response_class=ORJSONResponse, docs_url=None, redoc_url=None, openapi_url=None)
app.include_router(nvapi)

@app.get("/echo")
async def echo():
    return {}


@app.get(
    "/{api_type}/{server}/{sub_path:path}",
    response_model=None,
    dependencies=[Depends(validate_user_token(async_session))],
)
async def call_api(api_request: APIRequest = Depends(get_api_request)) -> Union[ORJSONResponse, StarletteResponse]:
    try:
        if api_request.api_type == APIType.api:
            response, status = await managers.get(SekaiServerRegion(api_request.server)).api_get(
                api_request.path, params=api_request.query_params
            )
            return ORJSONResponse(content=response, status_code=status)
        elif api_request.api_type == APIType.image:
            response, status = await managers.get(SekaiServerRegion(api_request.server)).image_get(api_request.path)
            return StarletteResponse(content=response, status_code=status, media_type="image/jpeg")
        raise HTTPException(status_code=400, detail="Invalid API type")
    except HTTPException:
        raise
    except Exception as e:
        traceback.print_exc()
        raise HTTPException(status_code=500, detail=f"Internal Server Error: {repr(e)}")
