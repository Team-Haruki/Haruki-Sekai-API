package api

type SekaiUser struct {
	ID         string `gorm:"column:id;type:varchar(64);primaryKey"`
	Credential string `gorm:"column:credential;type:varchar(128);not null"`
	Remark     string `gorm:"column:remark;type:varchar(255)"`
}

func (SekaiUser) TableName() string {
	return "sekai_users"
}

type SekaiUserServer struct {
	UserID string `gorm:"column:user_id;type:varchar(64);primaryKey"`
	Server string `gorm:"column:server;type:varchar(10);primaryKey"`
}

func (SekaiUserServer) TableName() string {
	return "sekai_user_servers"
}
