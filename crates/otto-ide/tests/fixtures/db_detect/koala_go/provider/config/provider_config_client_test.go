package config

// Expectations:
// - Error returned with message "error fetching provider configs from users service"
func TestGetAllConfigs_Error(t *testing.T) {
	assert.Equal(t, "error fetching provider configs from users service", err.Error())
}
