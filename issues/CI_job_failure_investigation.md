### Issue: CI Job Failure

**Repository**: Gameaday/ia-get-cli  
**Action Run**: [Job Logs](https://github.com/Gameaday/ia-get-cli/actions/runs/18048490006/job/51364956097)  
**Reference Commit**: baea6937689cb2e9f93d1454acf97fe827f8f775  

**Description**:  
The CI job is failing after a successful build and artifact copy, exiting with code 1. The logs suggest that a required 'toolchain' input may be missing, and there may be an unhandled 'exit 1' in the build script.

**Request**:  
1. Investigate and fix any missing required workflow inputs.  
2. Remove or guard against unconditional 'exit 1' statements in the build script.  
3. Add additional debug output at the end of the build steps to assist in troubleshooting.

**Logs**:  
Refer to the job logs for detailed error messages and context.