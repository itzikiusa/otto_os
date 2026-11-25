package models

type SessionInfo struct {
	ID int64 `gorm:"primaryKey"`
}

// TableName overrides the table name used by SessionInfo from the gorm default.
func (SessionInfo) TableName() string {
	return "session_info"
}

func (*Payout) TableName() string { return "payouts" }

func save(db *gorm.DB, p *Payout) {
	db.Table("payout_audit").Create(p)
	db.Model(p).Update("status", "paid")
}
