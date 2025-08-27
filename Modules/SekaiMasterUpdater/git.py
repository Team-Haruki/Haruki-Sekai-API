import git
import logging
import traceback
import coloredlogs

log_format = "[%(asctime)s][%(levelname)s][%(name)s] %(message)s"
field_style = {
    "asctime": {"color": "green"},
    "levelname": {"color": "blue", "bold": True},
    "name": {"color": "magenta"},
    "message": {"color": 144, "bright": False},
}
logger = logging.getLogger(__name__)
coloredlogs.install(level="DEBUG", logger=logger, fmt=log_format, field_styles=field_style)


class GitUpdater:
    def __init__(self, user: str, email: str, password: str, proxies: list) -> None:
        self.user = user
        self.email = email
        self.password = password
        self.proxy = proxies[0] if proxies else None

    def push_remote(self, repo: git.Repo, data_version: str) -> None:
        try:
            repo.git.add(A=True)
            diff = repo.index.diff("HEAD") if repo.head.is_valid() else repo.index.diff(None)
            if diff:
                author = git.Actor("Haruki Sekai Master Update Bot", "no-reply@seiunx.com")
                committer = git.Actor(self.user, self.email)
                message = f"master data version {data_version}"
                commit = repo.index.commit(message, author=author, committer=committer)
                branch = repo.active_branch.name
                remote = repo.remote("origin")
                original_url = remote.url

                if original_url.startswith("http://"):
                    url = original_url[len("http://"):]
                    scheme = "http://"
                elif original_url.startswith("https://"):
                    url = original_url[len("https://"):]
                    scheme = "https://"
                else:
                    url = original_url
                    scheme = ""
                if "@" in url:
                    url = url.split("@", 1)[1]

                auth_url = f"{scheme}{self.user}:{self.password}@{url}"
                remote.set_url(auth_url)
                env = {"https_proxy": self.proxy, "http_proxy": self.proxy} if self.proxy else None
                remote.push(refspec=f"{branch}:{branch}", env=env)
                remote.set_url(original_url)
                logger.info(f"Pushed to remote repository.")
            else:
                logger.info("No diffs to commit.")
        except Exception as e:
            traceback.print_exc()
            logger.error(f"Git occurred error while pushing to remote: {repr(e)}")
