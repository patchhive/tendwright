export function copilotCredentialMode(authenticated) {
  return authenticated ? "access_token" : null;
}
