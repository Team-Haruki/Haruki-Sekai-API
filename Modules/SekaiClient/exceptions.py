class SekaiClientException(Exception):
    """Sekai Client Base Exception"""

    pass


class SekaiAccountError(SekaiClientException):
    """User provided illegal accounts"""

    def __init__(self):
        super().__init__("You may not provide any correct accounts.")


class SessionError(SekaiClientException):
    """Account session error, you should re-login the account."""

    def __init__(self):
        super().__init__("Account session error")


class CookieExpiredError(SekaiClientException):
    """Sekai CloudFront cookie expired, you should refresh the cookies."""

    def __init__(self):
        super().__init__("Cookie expired.")


class UpdateRequiredError(SekaiClientException):
    """Game may be updated."""

    def __init__(self):
        super().__init__("UpdateRequiredError")


class UpgradeRequiredError(SekaiClientException):
    """App version may be upgraded, you should get a new app version."""

    def __init__(self):
        super().__init__("UpgradeRequiredError")


class UnderMaintenanceError(SekaiClientException):
    """Game server may under maintenance."""

    def __init__(self):
        super().__init__("Game server may under maintenance.")


class UnknownSekaiClientException(SekaiClientException):
    """Sekai Client occurred an unknown error."""

    def __init__(self, status_code, response):
        self.status_code = status_code
        self.response = response
