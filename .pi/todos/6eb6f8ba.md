{
  "id": "6eb6f8ba",
  "title": "Implement backend/macos.rs — security-framework backend",
  "tags": [],
  "status": "done",
  "created_at": "2026-04-01T17:13:27.204Z",
  "plan_id": "PLAN-a31b1564"
}

Implement PasswordBackend using security-framework crate. Use kSecClassGenericPassword with service "ansible-password-agent" and account as the type. Save with kSecUseAuthenticationWithBiometrics access control. Map errSecItemNotFound to Ok(None), handle errSecAuthFailed as user cancellation.
