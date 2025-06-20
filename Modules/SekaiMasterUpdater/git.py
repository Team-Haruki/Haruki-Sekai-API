import pygit2
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

    def push_remote(self, repo: pygit2.Repository, data_version: str) -> None:
        try:
            repo.index.add_all()
            repo.index.write()
            diff = repo.diff(repo.head.target, repo.index.write_tree())
            if diff.stats.insertions + diff.stats.deletions > 0:
                author = pygit2.Signature(self.user, self.email)
                committer = author
                tree = repo.index.write_tree()
                message = f"master data version {data_version}"
                ref = f"refs/heads/main"

                if repo.head_is_unborn:
                    parents = []
                else:
                    parents = [repo.head.target]

                oid = repo.create_commit(ref, author, committer, message, tree, parents)

                remote = repo.remotes["origin"]
                credentials = pygit2.UserPass(self.user, self.password)
                callbacks = pygit2.RemoteCallbacks(credentials=credentials)
                remote.push([f"+{ref}:{ref}"], callbacks=callbacks, proxy=self.proxy)
                logger.info(f"Pushed to remote repository.")
            else:
                logger.info("No changes to commit.")
        except Exception as e:
            traceback.print_exc()
            logger.error(f"Git occurred error while pushing to remote: {repr(e)}")
