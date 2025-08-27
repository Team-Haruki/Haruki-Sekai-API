import git
from pathlib import Path
from Modules.SekaiClient.model import (
    SekaiServerRegion,
    SekaiServerInfo,
    HarukiAssetUpdaterInfo,
    HarukiAppHashSource,
    HarukiAppHashSourceType,
)

JWT_SECRET = "SECRET_STRING"
DATABASE_URL = "sqlite+aiosqlite:///./test.db"
REDIS_HOST = "localhost"
REDIS_PORT = 6379
REDIS_PASSWORD = None
WORK_DIR = Path(__file__).parent  # Configure it if you need
PROXIES = ["http://127.0.0.1:7890"]  # Configure proxies here

# SekaiClient accounts directories configuration
ACCOUNTS_DIRS = {
    SekaiServerRegion.JP: WORK_DIR,
    SekaiServerRegion.TW: WORK_DIR,
    SekaiServerRegion.KR: WORK_DIR,
    SekaiServerRegion.EN: WORK_DIR,
    SekaiServerRegion.CN: WORK_DIR,
}

# Master data save directories configuration
MASTER_SAVE_DIRS = {
    SekaiServerRegion.JP: WORK_DIR,
    SekaiServerRegion.TW: WORK_DIR,
    SekaiServerRegion.KR: WORK_DIR,
    SekaiServerRegion.EN: WORK_DIR,
    SekaiServerRegion.CN: WORK_DIR,
}

# Version data save directories configuration
VERSION_SAVE_DIRS = {
    SekaiServerRegion.JP: WORK_DIR,
    SekaiServerRegion.TW: WORK_DIR,
    SekaiServerRegion.KR: WORK_DIR,
    SekaiServerRegion.EN: WORK_DIR,
    SekaiServerRegion.CN: WORK_DIR,
}

# Sekai server configuration
SEKAI_SERVERS = {
    SekaiServerRegion.JP: SekaiServerInfo(
        server=SekaiServerRegion.JP.value, api_url="", require_cookies=True, headers={}, aes_key=b"", aes_iv=b""
    ),
    SekaiServerRegion.EN: SekaiServerInfo(
        server=SekaiServerRegion.EN.value, api_url="", headers={}, aes_key=b"", aes_iv=b""
    ),
    SekaiServerRegion.TW: SekaiServerInfo(
        server=SekaiServerRegion.TW.value, api_url="", nuverse_master_data_url="", headers={}, aes_key=b"", aes_iv=b""
    ),
    SekaiServerRegion.KR: SekaiServerInfo(
        server=SekaiServerRegion.KR.value, api_url="", nuverse_master_data_url="", headers={}, aes_key=b"", aes_iv=b""
    ),
    SekaiServerRegion.CN: SekaiServerInfo(
        server=SekaiServerRegion.CN.value,
        enabled=False,  # CN server is disabled by default because it has not been online yet
        api_url="",
        nuverse_master_data_url="",
        headers={},
        aes_key=b"",
        aes_iv=b"",
    ),
}
SEKAI_APPHASH_SOURCES = [
    HarukiAppHashSource(type_=HarukiAppHashSourceType.FILE, dir=Path("dir/to/source")),
    HarukiAppHashSource(type_=HarukiAppHashSourceType.URL, url="https://source.com/apphash/"),
]

# Master data Git repositories configuration
ENABLE_GIT_PUSH = False  # Set to True if you need to update the Git master repository. Keep it False if not needed.
GIT_USER = "<NAME>"  # Git user name
GIT_EMAIL = "<EMAIL>"  # Git user email
GIT_PASS = "<PAT>"  # GitHub Personal Access Token (PAT)
REPOS = {server: git.Repo(Path(master_dir).parent) for server, master_dir in MASTER_SAVE_DIRS.items()}

# Logger configuration
LOG_FORMAT = "[%(asctime)s][%(levelname)s][%(name)s] %(message)s"
FIELD_STYLE = {
    "asctime": {"color": "green"},
    "levelname": {"color": "blue", "bold": True},
    "name": {"color": "magenta"},
    "message": {"color": 144, "bright": False},
}
# External asset updater URL
ASSET_UPDATER_SERVERS = [HarukiAssetUpdaterInfo(url="http://127.0.0.1:12345")]
