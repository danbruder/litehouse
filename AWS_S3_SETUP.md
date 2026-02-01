# AWS S3 Setup for Litestream Integration

This guide describes how to set up minimal S3 permissions for litestream database backups in litehouse.

## Quick Reference: Minimal IAM Policy

Copy this policy when creating an IAM user for litestream S3 integration:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "LitestreamBackupReplication",
      "Effect": "Allow",
      "Action": [
        "s3:PutObject",
        "s3:GetObject",
        "s3:DeleteObject"
      ],
      "Resource": "arn:aws:s3:::BUCKET_NAME/backup/*"
    },
    {
      "Sid": "LitestreamListBucket",
      "Effect": "Allow",
      "Action": "s3:ListBucket",
      "Resource": "arn:aws:s3:::BUCKET_NAME",
      "Condition": {
        "StringLike": {
          "s3:prefix": "backup/*"
        }
      }
    },
    {
      "Sid": "GetBucketLocation",
      "Effect": "Allow",
      "Action": "s3:GetBucketLocation",
      "Resource": "arn:aws:s3:::BUCKET_NAME"
    }
  ]
}
```

## Setup Instructions

### 1. Create S3 Bucket

```bash
aws s3 mb s3://your-bucket-name --region us-east-1
```

### 2. Create IAM User

```bash
aws iam create-user --user-name litehouse-s3-user
```

### 3. Create IAM Policy

1. Replace `BUCKET_NAME` in the policy above with your actual bucket name
2. Create the policy:

```bash
aws iam create-policy \
  --policy-name LitehouseS3BackupPolicy \
  --policy-document file://policy.json
```

### 4. Attach Policy to User

```bash
aws iam attach-user-policy \
  --user-name litehouse-s3-user \
  --policy-arn arn:aws:iam::YOUR_ACCOUNT_ID:policy/LitehouseS3BackupPolicy
```

### 5. Create Access Key

```bash
aws iam create-access-key --user-name litehouse-s3-user
```

Save the `AccessKeyId` and `SecretAccessKey` - you'll need these for litestream configuration.

### 6. Configure Litehouse

Store the credentials in the litehouse database and configure the litestream container to use them via environment variables.

## Permissions Explained

| Permission | Purpose | Why Needed |
|-----------|---------|-----------|
| `s3:PutObject` | Upload database transaction logs (.ltx files) | Litestream writes backup data |
| `s3:GetObject` | Read backups during restore operations | Restore from S3 backup |
| `s3:DeleteObject` | Remove old files during compaction | Litestream cleanup/optimization |
| `s3:ListBucket` | List objects in bucket | Litestream checks what exists |
| `s3:GetBucketLocation` | Determine bucket region | Needed for API calls |

The `backup/*` prefix restriction ensures the user can only access files under the `backup/` directory, not the entire bucket.

## Verification

After setup, verify backups are working:

```bash
aws s3 ls s3://your-bucket-name/backup --recursive
```

You should see files like:
```
backup/main/db/0000/0000000000000001-0000000000000001.ltx
backup/apps/{app-id}/app.db/0000/0000000000000001-0000000000000001.ltx
```

## Troubleshooting

### 403 AccessDenied Errors

1. **Check bucket name** - Verify the bucket name in the policy matches what litestream is using
2. **Verify policy attachment** - Confirm the policy is attached to the user: `aws iam list-user-policies --user-name litehouse-s3-user`
3. **Check credentials** - Verify AccessKeyId and SecretAccessKey are correct in the litestream config
4. **Wait for IAM propagation** - IAM changes can take a minute to propagate
5. **Check litestream logs** - See error details: `docker logs litestream-container | grep -i error`

### Backups Not Appearing

1. Verify litestream container is running: `docker ps | grep litestream`
2. Check for connection errors: `docker logs litestream-container`
3. Verify S3 bucket exists and is accessible: `aws s3 ls s3://your-bucket-name/`
4. Check that app databases exist: Check litehouse database for registered apps

## Security Best Practices

- ✅ Use a dedicated IAM user for litestream (not root credentials)
- ✅ Use minimal permissions (scoped to `backup/*` prefix)
- ✅ Enable S3 bucket encryption at rest
- ✅ Enable S3 versioning for additional protection
- ✅ Consider enabling MFA delete on the bucket
- ✅ Rotate access keys periodically (e.g., every 90 days)
- ✅ Use bucket policies to prevent accidental public access
- ✅ Enable CloudTrail logging for audit trail

## References

- [Litestream S3 Documentation](https://litestream.io/guides/s3/)
- [AWS S3 Security Best Practices](https://docs.aws.amazon.com/AmazonS3/latest/userguide/security-best-practices.html)
- [AWS IAM Best Practices](https://docs.aws.amazon.com/IAM/latest/userguide/best-practices.html)
