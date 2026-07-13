use std::ffi::OsStr;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::sync::Arc;

use super::*;

mod sys;
use sys::*;

#[derive(Debug)]
pub struct FsRoot {
    spec: FsRootSpec,
    shared_fd: OwnedFd,
    private_fd: OwnedFd,
    shared_identity: ObjectIdentity,
}

#[derive(Debug, Clone)]
pub struct PreparedObject {
    pub object_id: ObjectId,
    pub private_name: String,
    pub identity: ObjectIdentity,
    fd: Arc<OwnedFd>,
}

#[derive(Debug, Clone)]
pub struct PublishedObject {
    pub object_id: ObjectId,
    pub leaf: LeafName,
    pub identity: ObjectIdentity,
    fd: Arc<OwnedFd>,
}

#[derive(Debug, Clone)]
pub struct QuarantinedObject {
    pub object_id: ObjectId,
    pub private_name: String,
    pub identity: ObjectIdentity,
    fd: Arc<OwnedFd>,
}

impl FsRoot {
    pub fn open(spec: FsRootSpec) -> Result<Self, FsTxError> {
        validate_spec(&spec)?;
        let common_fd = open_absolute_directory(&spec.common_ancestor)?;
        let common_stat = fstat(common_fd.as_raw_fd(), "common_ancestor")?;
        validate_directory("common_ancestor", &common_stat, spec.broker_uid, None, None)?;
        if common_stat.st_mode as u32 & 0o022 != 0 {
            return Err(FsTxError::InvalidSpec(
                "common_ancestor must not be group- or other-writable".into(),
            ));
        }
        let shared_relative = relative_to(&spec.common_ancestor, &spec.shared_path)?;
        let private_relative = relative_to(&spec.common_ancestor, &spec.private_path)?;
        let shared_fd = open_directory_at(common_fd.as_raw_fd(), shared_relative)?;
        let private_fd = open_directory_at(common_fd.as_raw_fd(), private_relative)?;
        let shared_stat = fstat(shared_fd.as_raw_fd(), "shared")?;
        let private_stat = fstat(private_fd.as_raw_fd(), "private")?;
        validate_directory(
            "shared",
            &shared_stat,
            spec.broker_uid,
            Some(spec.broker_gid),
            Some(spec.shared_mode),
        )?;
        validate_directory(
            "private",
            &private_stat,
            spec.broker_uid,
            Some(spec.broker_gid),
            Some(spec.private_mode),
        )?;
        if shared_stat.st_dev != private_stat.st_dev {
            return Err(FsTxError::InvalidSpec(
                "shared and private roots are not on the same mount".into(),
            ));
        }
        if shared_stat.st_dev == private_stat.st_dev && shared_stat.st_ino == private_stat.st_ino {
            return Err(FsTxError::InvalidSpec(
                "shared and private roots resolve to the same directory".into(),
            ));
        }
        probe_rename_noreplace(shared_fd.as_raw_fd())?;
        probe_openat2(shared_fd.as_raw_fd())?;
        let shared_identity = identity_at(shared_fd.as_raw_fd(), cstr_dot())?;
        probe_open_by_handle(shared_fd.as_raw_fd(), &shared_identity)?;
        Ok(Self {
            spec,
            shared_fd,
            private_fd,
            shared_identity,
        })
    }

    pub fn alias(&self) -> &RootAlias {
        &self.spec.alias
    }

    pub fn root_identity(&self) -> &ObjectIdentity {
        &self.shared_identity
    }

    pub fn inspect_shared(&self, leaf: &LeafName) -> Result<Option<ObjectIdentity>, FsTxError> {
        let name = cstring(leaf.as_str())?;
        identity_optional_at(self.shared_fd.as_raw_fd(), &name)
    }

    pub fn inspect_private(&self, name: &str) -> Result<Option<ObjectIdentity>, FsTxError> {
        if name.is_empty() || matches!(name, "." | "..") || name.as_bytes().contains(&b'/') {
            return Err(FsTxError::InvalidSpec(
                "private inspection name must be one non-special component".into(),
            ));
        }
        let name = cstring(name)?;
        identity_optional_at(self.private_fd.as_raw_fd(), &name)
    }

    /// Re-establishes the durability fence for a previously completed
    /// private-namespace mutation. Recovery uses this before committing an
    /// observed `quarantined -> deleted` kill window in SQLite.
    pub fn sync_private(&self) -> Result<(), FsTxError> {
        sync_fd(
            self.private_fd.as_raw_fd(),
            "fsync private recovery fence",
            "private",
        )
    }

    pub fn prepare(&self, object_id: ObjectId) -> Result<PreparedObject, FsTxError> {
        let name = format!("p-{object_id}");
        let c_name = cstring(&name)?;
        mkdirat(self.private_fd.as_raw_fd(), &c_name, 0o700, &name)?;
        let result = (|| {
            let fd = open_directory_at(self.private_fd.as_raw_fd(), OsStr::new(&name))?;
            let before = identity_at(self.private_fd.as_raw_fd(), &c_name)?;
            probe_open_by_handle(self.private_fd.as_raw_fd(), &before)?;
            chown_mode(
                fd.as_raw_fd(),
                self.spec.published_uid,
                self.spec.published_gid,
                self.spec.published_mode,
                &name,
            )?;
            sync_fd(fd.as_raw_fd(), "fsync prepared object", &name)?;
            sync_fd(self.private_fd.as_raw_fd(), "fsync private root", "private")?;
            let identity = identity_at(self.private_fd.as_raw_fd(), &c_name)?;
            if !identity.same_authority(&before) {
                return Err(mismatch(
                    &name,
                    before,
                    identity,
                    MismatchDisposition::PreservedPrivate {
                        quarantine_name: name.clone(),
                    },
                ));
            }
            Ok(PreparedObject {
                object_id,
                private_name: name.clone(),
                identity,
                fd: Arc::new(fd),
            })
        })();
        if result.is_err() {
            let _ = unlinkat_dir(self.private_fd.as_raw_fd(), &c_name, &name);
            let _ = sync_fd(
                self.private_fd.as_raw_fd(),
                "fsync private rollback",
                "private",
            );
        }
        result
    }

    pub fn reopen_prepared(
        &self,
        object_id: ObjectId,
        expected: &ObjectIdentity,
    ) -> Result<PreparedObject, FsTxError> {
        let private_name = format!("p-{object_id}");
        let fd = self.reopen_at(
            self.private_fd.as_raw_fd(),
            &private_name,
            expected,
            MismatchDisposition::PreservedPrivate {
                quarantine_name: private_name.clone(),
            },
        )?;
        Ok(PreparedObject {
            object_id,
            private_name,
            identity: expected.clone(),
            fd: Arc::new(fd),
        })
    }

    pub fn reopen_published(
        &self,
        object_id: ObjectId,
        leaf: LeafName,
        expected: &ObjectIdentity,
    ) -> Result<PublishedObject, FsTxError> {
        let fd = self.reopen_at(
            self.shared_fd.as_raw_fd(),
            leaf.as_str(),
            expected,
            MismatchDisposition::UnchangedShared,
        )?;
        Ok(PublishedObject {
            object_id,
            leaf,
            identity: expected.clone(),
            fd: Arc::new(fd),
        })
    }

    pub fn reopen_quarantined(
        &self,
        object_id: ObjectId,
        private_name: &str,
        expected: &ObjectIdentity,
    ) -> Result<QuarantinedObject, FsTxError> {
        let required = format!("q-{object_id}");
        if private_name != required {
            return Err(FsTxError::InvalidSpec(format!(
                "quarantine name {private_name:?} does not match {required:?}"
            )));
        }
        let fd = self.reopen_at(
            self.private_fd.as_raw_fd(),
            private_name,
            expected,
            MismatchDisposition::PreservedPrivate {
                quarantine_name: private_name.into(),
            },
        )?;
        Ok(QuarantinedObject {
            object_id,
            private_name: private_name.into(),
            identity: expected.clone(),
            fd: Arc::new(fd),
        })
    }

    pub fn publish(
        &self,
        prepared: &PreparedObject,
        leaf: LeafName,
    ) -> Result<PublishedObject, FsTxError> {
        self.verify_fd(&prepared.fd, &prepared.identity, &prepared.private_name)?;
        let source = cstring(&prepared.private_name)?;
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
            "fsync private after publish",
            "private",
        )?;
        sync_fd(
            self.shared_fd.as_raw_fd(),
            "fsync shared after publish",
            "shared",
        )?;
        let observed = identity_at(self.shared_fd.as_raw_fd(), &destination)?;
        if !observed.same_authority(&prepared.identity) {
            return Err(mismatch(
                leaf.as_str(),
                prepared.identity.clone(),
                observed,
                MismatchDisposition::PreservedOpenHandle,
            ));
        }
        Ok(PublishedObject {
            object_id: prepared.object_id.clone(),
            leaf,
            identity: observed,
            fd: Arc::clone(&prepared.fd),
        })
    }

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

    fn verify_fd(
        &self,
        fd: &OwnedFd,
        expected: &ObjectIdentity,
        path: &str,
    ) -> Result<(), FsTxError> {
        let stat = fstat(fd.as_raw_fd(), path)?;
        if stat.st_dev as u64 != expected.device || stat.st_ino as u64 != expected.inode {
            return Err(FsTxError::CapabilityUnavailable {
                capability: "stable open object descriptor",
                detail: format!("descriptor identity changed for {path}"),
            });
        }
        probe_open_by_handle(self.shared_fd.as_raw_fd(), expected)
    }

    fn reopen_at(
        &self,
        parent: RawFd,
        name: &str,
        expected: &ObjectIdentity,
        disposition: MismatchDisposition,
    ) -> Result<OwnedFd, FsTxError> {
        let c_name = cstring(name)?;
        let observed = identity_at(parent, &c_name)?;
        if !observed.same_authority(expected) {
            return Err(mismatch(name, expected.clone(), observed, disposition));
        }
        probe_open_by_handle(self.shared_fd.as_raw_fd(), expected)?;
        let fd = open_directory_at(parent, OsStr::new(name))?;
        let stat = fstat(fd.as_raw_fd(), name)?;
        if stat.st_dev as u64 != expected.device || stat.st_ino as u64 != expected.inode {
            return Err(FsTxError::CapabilityUnavailable {
                capability: "recovery descriptor identity",
                detail: format!("descriptor changed while reopening {name}"),
            });
        }
        Ok(fd)
    }
}
