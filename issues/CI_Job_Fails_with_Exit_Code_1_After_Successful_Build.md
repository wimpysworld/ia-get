### CI Job Fails with Exit Code 1 After Successful Build

**Description:**
The CI workflow fails with exit code 1 after successfully building and copying the aarch64-linux-android artifact. The job logs indicate the following:

- Build completes successfully: `✓ Successfully built for aarch64-linux-android`
- Artifact is copied: `✓ Copied to arm64-v8a directory`
- Job then fails: `Process completed with exit code 1.`

Additional context from the logs:
- There is a message earlier in the logs: `'toolchain' is a required input`
- There may be an unhandled `exit 1` in the build script or a missing required file/input at the end of the job.

**Steps to Reproduce:**
1. Run the CI workflow on commit [baea693](https://github.com/Gameaday/ia-get-cli/tree/baea6937689cb2e9f93d1454acf97fe827f8f775).
2. Observe the final build and copy steps.
3. See the job fail with exit code 1.

**Job Reference:**
[CI Job Run/Logs](https://github.com/Gameaday/ia-get-cli/actions/runs/18048490006/job/51364956097) (ref: baea6937689cb2e9f93d1454acf97fe827f8f775)

**Recommended Investigation and Fixes:**
- Ensure the `toolchain` input is set where required in the workflow or build scripts.
- Review all `exit 1` statements at the end of build scripts; guard them or remove unconditional failures.
- Add debug output to verify the presence of required files/variables after build steps.
- Verify all workflow job environment variables and secrets are correctly configured.

**Logs:**
See the job logs for detailed error context.