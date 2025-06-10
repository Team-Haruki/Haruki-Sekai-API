import asyncio
import traceback
from pathlib import Path
from typing import Tuple, Union
from quart import Blueprint, Response, jsonify, request

from Modules.SekaiClient.model import SekaiServerRegion
from utils import managers

api = Blueprint("api_proxy", __name__)
if (Path(__file__).parent / "nuverse.py").exists():
    from nuverse import nvapi

    api.register_blueprint(nvapi)


@api.before_app_serving
async def _managers_init() -> None:
    tasks = [manager.init() for _, manager in managers.items()]
    await asyncio.gather(*tasks)


@api.after_app_serving
async def _managers_shutdown() -> None:
    tasks = [manager.shutdown() for _, manager in managers.items()]
    await asyncio.gather(*tasks)


@api.route("/<api_type>/<server>/<path:sub_path>", methods=["GET"])
async def _call_api(api_type, server, sub_path) -> Union[Tuple[Response, int], Response]:
    try:
        params = request.args if request.args else None
        sub_path = "/" + sub_path
        if api_type == "api":
            response, status = await managers.get(SekaiServerRegion(server)).api_get(sub_path, params=params)
            return jsonify(response), status
        elif api_type == "image":
            response, status = await managers.get(SekaiServerRegion(server)).image_get(sub_path)
            return Response(response, status, content_type="image/jpeg")
        else:
            return jsonify({"error": "Invalid API type"}), 400
    except Exception as e:
        traceback.print_exc()
        return jsonify({"error": "Internal Server Error", "repr": f"{repr(e)}"}), 500
