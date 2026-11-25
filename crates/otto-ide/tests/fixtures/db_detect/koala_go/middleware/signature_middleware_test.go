package middleware

// Test description: Middleware validates the HMAC signature
//
// Preparation:
// - Base64-encoded valid secret and HMAC signature computed from path
//
// Expectations:
// - Invalid HMAC signature returns 401 Unauthorized with "invalid signature"
func TestMiddleware_Signature(t *testing.T) {}

// Test description: New with Fetcher should panic when provider config initialization fails
//
// Preparation:
// - Mock client returns error from GetAllConfigs
func TestMiddleware_PanicsOnInit(t *testing.T) {}

// Expectations:
// - Middleware resolves secret from provider config cache
func TestMiddleware_WithProviderConfigFromCache(t *testing.T) {}
