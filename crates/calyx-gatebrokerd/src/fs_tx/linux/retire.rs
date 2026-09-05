//! Retirement authority for published objects: quarantine, verified deletion,
//! prepared-object rollback, and restore. Every operation re-checks object
//! identity against the held descriptor before and after the mutation and
//! fences each rename or unlink with directory fsyncs.

use std::os::fd::AsRawFd;
use std::sync::Arc;

use super::*;

impl FsRoot {
    pub fn quarantine(&self, published: &PublishedObject) -> Result<QuarantinedObject, FsTxError> {
        let leaf = cstring(published.leaf.as_str())?;
        let private_name = format!("q-{}", published.object_id);
        let destination = cstring(&private_name)?;
        rename_noreplace(
            self.shared_fd.as_raw_fd(),
            &leaf,
            self.private_fd.as_raw_fd(),
            &destination,
            &private_name,
        )?;
        sync_fd(
            self.shared_fd.as_raw_fd(),
            "fsync shared after quarantine",
            "shared",
        )?;
        sync_fd(
            self.private_fd.as_raw_fd(),
            "fsync private after quarantine",
            "private",
        )?;
        let observed = identity_at(self.private_fd.as_raw_fd(), &destination)?;
        if !observed.same_authority(&published.identity) {
            let restored = rename_noreplace(
                self.private_fd.as_raw_fd(),
                &destination,
                self.shared_fd.as_raw_fd(),
                &leaf,
                published.leaf.as_str(),
            )
            .is_ok();
            if restored {
                sync_fd(
                    self.private_fd.as_raw_fd(),
                    "fsync private after mismatch restore",
                    "private",
                )?;
                sync_fd(
                    self.shared_fd.as_raw_fd(),
                    "fsync shared after mismatch restore",
                    "shared",
                )?;
            }
            let disposition = if restored {
                MismatchDisposition::RestoredShared
            } else {
                MismatchDisposition::PreservedPrivate {
                    quarantine_name: private_name,
                }
            };
            return Err(mismatch(
                published.leaf.as_str(),
                published.identity.clone(),
                observed,
                disposition,
            ));
        }
        self.verify_fd(&published.fd, &published.identity, published.leaf.as_str())?;
        chown_mode(
            published.fd.as_raw_fd(),
            self.spec.broker_uid,
            self.spec.broker_gid,
            0o700,
            &private_name,
        )?;
        sync_fd(
            published.fd.as_raw_fd(),
            "fsync quarantined object",
            &private_name,
        )?;
        Ok(QuarantinedObject {
            object_id: published.object_id.clone(),
            private_name,
            identity: identity_at(self.private_fd.as_raw_fd(), &destination)?,
            fd: Arc::clone(&published.fd),
        })
    }

    pub fn delete_quarantined(&self, object: &QuarantinedObject) -> Result<(), FsTxError> {
        self.verify_fd(&object.fd, &object.identity, &object.private_name)?;
        let mut count = 0;
        delete_contents(object.fd.as_raw_fd(), 0, &mut count)?;
        let name = cstring(&object.private_name)?;
        let observed = identity_at(self.private_fd.as_raw_fd(), &name)?;
        if !observed.same_authority(&object.identity) {
            return Err(mismatch(
                &object.private_name,
                object.identity.clone(),
                observed,
                MismatchDisposition::PreservedPrivate {
                    quarantine_name: object.private_name.clone(),
                },
            ));
        }
        unlinkat_dir(self.private_fd.as_raw_fd(), &name, &object.private_name)?;
        sync_fd(
            self.private_fd.as_raw_fd(),
            "fsync private after delete",
            "private",
        )
    }

    pub fn discard_prepared(&self, object: &PreparedObject) -> Result<(), FsTxError> {
        self.verify_fd(&object.fd, &object.identity, &object.private_name)?;
        let mut count = 0;
        delete_contents(object.fd.as_raw_fd(), 0, &mut count)?;
        let name = cstring(&object.private_name)?;
        let observed = identity_at(self.private_fd.as_raw_fd(), &name)?;
        if !observed.same_authority(&object.identity) {
            return Err(mismatch(
                &object.private_name,
                object.identity.clone(),
                observed,
                MismatchDisposition::PreservedPrivate {
                    quarantine_name: object.private_name.clone(),
                },
            ));
        }
        unlinkat_dir(self.private_fd.as_raw_fd(), &name, &object.private_name)?;
        sync_fd(
            self.private_fd.as_raw_fd(),
            "fsync private after prepared rollback",
            "private",
        )
    }

    pub fn quarantine_prepared(
        &self,
        object: &PreparedObject,
    ) -> Result<QuarantinedObject, FsTxError> {
        self.verify_fd(&object.fd, &object.identity, &object.private_name)?;
        let destination_name = format!("q-{}", object.object_id);
        let source = cstring(&object.private_name)?;
        let destination = cstring(&destination_name)?;
        rename_noreplace(
            self.private_fd.as_raw_fd(),
            &source,
            self.private_fd.as_raw_fd(),
            &destination,
            &destination_name,
        )?;
        sync_fd(
            self.private_fd.as_raw_fd(),
            "fsync private after prepared quarantine",
            "private",
        )?;
        let observed = identity_at(self.private_fd.as_raw_fd(), &destination)?;
        if !observed.same_authority(&object.identity) {
            return Err(mismatch(
                &destination_name,
                object.identity.clone(),
                observed,
                MismatchDisposition::PreservedPrivate {
                    quarantine_name: destination_name.clone(),
                },
            ));
        }
        chown_mode(
            object.fd.as_raw_fd(),
            self.spec.broker_uid,
            self.spec.broker_gid,
            0o700,
            &destination_name,
        )?;
        sync_fd(
            object.fd.as_raw_fd(),
            "fsync prepared quarantine object",
            &destination_name,
        )?;
        Ok(QuarantinedObject {
            object_id: object.object_id.clone(),
            private_name: destination_name,
            identity: identity_at(self.private_fd.as_raw_fd(), &destination)?,
            fd: Arc::clone(&object.fd),
        })
    }

    pub fn restore(
        &self,
        object: &QuarantinedObject,
        leaf: LeafName,
    ) -> Result<PublishedObject, FsTxError> {
        self.verify_fd(&object.fd, &object.identity, &object.private_name)?;
        chown_mode(
            object.fd.as_raw_fd(),
            self.spec.published_uid,
            self.spec.published_gid,
            self.spec.published_mode,
            &object.private_name,
        )?;
        let source = cstring(&object.private_name)?;
        let destination = cstring(leaf.as_str())?;
        rename_noreplace(
            self.private_fd.as_raw_fd(),
            &source,
            self.shared_fd.as_raw_fd(),
            &destination,
            leaf.as_str(),
        )?;
        sync_fd(
            self.private_fd.as_raw_fd(),
            "fsync private after restore",
            "private",
        )?;
        sync_fd(
            self.shared_fd.as_raw_fd(),
            "fsync shared after restore",
            "shared",
        )?;
        Ok(PublishedObject {
            object_id: object.object_id.clone(),
            leaf,
            identity: identity_at(self.shared_fd.as_raw_fd(), &destination)?,
            fd: Arc::clone(&object.fd),
        })
    }
}
