import sys
import asyncio
import platform
from quart import Quart
from hypercorn.asyncio import serve
from hypercorn.config import Config
from apscheduler.triggers.cron import CronTrigger
from apscheduler.schedulers.asyncio import AsyncIOScheduler

from configs import HOST, PORT
from core import check_update
from api import api

app = Quart(__name__)
app.register_blueprint(api)
scheduler = AsyncIOScheduler()


@app.before_serving
async def _init() -> None:
    scheduler.add_job(check_update, CronTrigger(second=7))
    scheduler.start()


@app.after_serving
async def _shutdown() -> None:
    scheduler.shutdown()


async def run() -> None:
    config = Config()
    config.bind = [f'{HOST}:{PORT}']
    await serve(app, config)


if __name__ == '__main__':
    if platform.system() == 'Windows':
        asyncio.run(run())
    else:
        import uvloop

        python_version = sys.version_info
        if python_version >= (3, 11):
            uvloop.run(run())
        else:
            uvloop.install()
            asyncio.run(run())
