# Windows Code Signing Setup

This document describes the Windows code signing implementation for ia-get CLI binaries to prevent security warnings when users download and run the executable.

## Overview

The project uses Azure Trusted Signing service integrated with GitHub Actions to automatically sign Windows executables during the release process. This eliminates Windows SmartScreen warnings and provides users with confidence in the authenticity of the binaries.

## Implementation

### GitHub Actions Integration

The signing is implemented in `.github/workflows/release.yml` using the `azure/trusted-signing@v3` action. The signing occurs:

1. **After** the Windows binary is built (`cargo build --release --target x86_64-pc-windows-msvc`)
2. **Before** the binary is packaged into the release ZIP file
3. **Only** for Windows targets (`x86_64-pc-windows-msvc` on `windows-latest` runners)

### Workflow Steps

1. **Build**: Standard Rust compilation for Windows target
2. **Sign**: Azure Trusted Signing signs the `ia-get.exe` binary
3. **Verify**: PowerShell script validates the signature
4. **Package**: Signed binary is packaged into release artifacts

## Required GitHub Secrets

The following secrets must be configured in the GitHub repository settings:

| Secret Name | Description | Required |
|-------------|-------------|----------|
| `AZURE_TENANT_ID` | Azure Active Directory tenant ID | ✅ |
| `AZURE_CLIENT_ID` | Azure service principal client ID | ✅ |
| `AZURE_CLIENT_SECRET` | Azure service principal client secret | ✅ |
| `AZURE_TRUSTED_SIGNING_ENDPOINT` | Azure Trusted Signing service endpoint | ✅ |
| `AZURE_CODE_SIGNING_ACCOUNT_NAME` | Code signing account name in Azure | ✅ |
| `AZURE_CERTIFICATE_PROFILE_NAME` | Certificate profile name for signing | ✅ |

## Azure Trusted Signing Setup

### Prerequisites

1. **Azure Subscription**: Active Azure subscription with permissions to create resources
2. **Azure Trusted Signing Service**: Enabled in your Azure subscription
3. **Code Signing Certificate**: Valid code signing certificate or Azure-managed certificate
4. **Service Principal**: Azure AD service principal with appropriate permissions

### Setup Steps

1. **Create Azure Trusted Signing Account**:
   ```bash
   az extension add --name trustedsigning
   az trustedsigning account create \
     --account-name "ia-get-signing" \
     --resource-group "your-resource-group" \
     --location "EastUS"
   ```

2. **Create Certificate Profile**:
   ```bash
   az trustedsigning certificate-profile create \
     --account-name "ia-get-signing" \
     --profile-name "ia-get-profile" \
     --resource-group "your-resource-group" \
     --profile-type "PublicTrust" \
     --subject "CN=Your Organization Name,O=Your Organization,C=US"
   ```

3. **Create Service Principal**:
   ```bash
   az ad sp create-for-rbac \
     --name "ia-get-signing-sp" \
     --role "Trusted Signing Certificate Profile Signer" \
     --scopes "/subscriptions/{subscription-id}/resourceGroups/{resource-group}/providers/Microsoft.CodeSigning/accounts/ia-get-signing"
   ```

4. **Configure GitHub Secrets**: Add the service principal credentials and signing service details to GitHub repository secrets.

## Verification

The workflow includes automatic signature verification using PowerShell's `Get-AuthenticodeSignature` cmdlet. The verification checks:

- ✅ Signature status is "Valid"
- ✅ Certificate subject and thumbprint
- ✅ Timestamp information
- ✅ Certificate chain validation

## Benefits

- **User Trust**: Eliminates Windows SmartScreen warnings
- **Security**: Provides cryptographic proof of binary authenticity
- **Compliance**: Meets enterprise security requirements
- **Automation**: No manual intervention required for releases

## Troubleshooting

### Common Issues

1. **Signature Verification Failed**:
   - Check Azure service principal permissions
   - Verify certificate profile configuration
   - Ensure certificate is not expired

2. **Binary Not Found**:
   - Check Rust build process completed successfully
   - Verify target directory structure
   - Ensure Windows runner has correct file paths

3. **Azure Authentication Failed**:
   - Verify GitHub secrets are correctly configured
   - Check Azure service principal credentials
   - Ensure Azure Trusted Signing service is accessible

### Debug Information

The workflow provides detailed logging for debugging:
- Certificate subject and thumbprint
- Signature timestamp
- File paths and binary detection
- Azure authentication status

## Security Considerations

1. **Secret Management**: GitHub secrets are encrypted and only accessible to authorized workflows
2. **Certificate Rotation**: Azure Trusted Signing handles certificate lifecycle management
3. **Audit Trail**: All signing operations are logged in Azure Activity Log
4. **Access Control**: Service principal has minimal required permissions

## Cost Considerations

Azure Trusted Signing is a paid service. Costs include:
- Per-signature fees for code signing operations
- Certificate management and storage
- Service principal and Azure AD usage

For open-source projects, consider applying for Microsoft's sponsorship programs that may cover signing costs.

## References

- [Azure Trusted Signing Documentation](https://docs.microsoft.com/en-us/azure/trusted-signing/)
- [azure/trusted-signing GitHub Action](https://github.com/Azure/trusted-signing)
- [Windows Code Signing Best Practices](https://docs.microsoft.com/en-us/windows/win32/seccrypto/cryptography-tools)