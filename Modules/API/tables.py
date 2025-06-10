from sqlalchemy.orm import DeclarativeBase
from sqlalchemy import Column, String, ForeignKey


class Base(DeclarativeBase):
    pass


class SekaiUser(Base):
    __tablename__ = "sekai_users"
    id = Column(String(64), primary_key=True)
    credential = Column(String(128), nullable=False)
    remark = Column(String(255))


class SekaiUserServer(Base):
    __tablename__ = "sekai_user_servers"
    user_id = Column(String(64), ForeignKey("sekai_users.id"), primary_key=True)
    server = Column(String(10), primary_key=True)
