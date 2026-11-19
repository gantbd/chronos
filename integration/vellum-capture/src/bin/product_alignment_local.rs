#![forbid(unsafe_code)]

#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    eprintln!("Local product alignment requires macOS.");
    std::process::ExitCode::FAILURE
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    macos::run()
}

#[cfg(target_os = "macos")]
mod macos {
    use chronos_client::AlignmentWireClientV1;
    use chronos_protocol::alignment_v1 as public;
    use chronos_sdk_embedded::engine::{
        RevisionedTaskSnapshotV1, StateRevision, TaskPhase, TaskRefV1, TaskSnapshotV1,
    };
    use chronos_vellum_capture::{
        SourceReferenceV1,
        alignment::{
            AlignmentBasisV1, AlignmentCohortV1, AlignmentDisclosureV1, AlignmentLimitsV1,
            AlignmentRecordV1, AlignmentRegisterErrorV1, AlignmentRegisterV1, GroomingProposalV1,
            GroomingRecommendationV1, OwnerEvidenceRefV1, PrincipalRefV1,
            journal::DirectoryAlignmentJournalV1,
            local_runtime::{
                LocalAlignmentBindingV1, LocalAlignmentConnectionsV1,
                LocalProductAlignmentRuntimeV1, observe_local_vellum_artifact_v1,
                sample_local_product_alignment_v1,
            },
        },
    };
    use moira_protocol::deliberation::DeliberationIdV1;
    use serde::Deserialize;
    use std::{
        fs,
        net::SocketAddr,
        path::{Path, PathBuf},
        time::Duration,
    };

    const MAX_CONFIG_BYTES: u64 = 64 * 1024;
    const MAX_TOKEN_BYTES: u64 = 8 * 1024;
    const LIMITS: AlignmentLimitsV1 = AlignmentLimitsV1 {
        max_field_bytes: 4096,
        max_evidence: 64,
        max_next_tasks: 32,
        max_record_bytes: 256 * 1024,
    };

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ConfigV1 {
        version: u8,
        chronos_directory: PathBuf,
        moira_socket: PathBuf,
        vellum_socket: PathBuf,
        vellum_credential: PathBuf,
        tellus_endpoint: String,
        tellus_token_file: PathBuf,
        register_directory: PathBuf,
        listener_directory: PathBuf,
        timeout_ms: u64,
        max_moira_frame_bytes: usize,
        chronos_scope: String,
        deliberation: String,
        external_advocate: StakeholderBindingV1,
        product_manager: StakeholderBindingV1,
        frontier_supervisor: StakeholderBindingV1,
        goal: ArtifactBindingV1,
        proposal: ArtifactBindingV1,
        budget_policy: BudgetPolicyBindingV1,
        control_task: TaskBindingV1,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StakeholderBindingV1 {
        stakeholder: String,
        principal: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ArtifactBindingV1 {
        locator: String,
        namespace: String,
        identity: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct BudgetPolicyBindingV1 {
        locator: String,
        resource: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TaskBindingV1 {
        task: String,
        incarnation: u64,
    }

    struct ExactScopeDisclosure(String);

    impl AlignmentDisclosureV1 for ExactScopeDisclosure {
        fn allows(&mut self, record: &AlignmentRecordV1, current: &AlignmentBasisV1) -> bool {
            record.proposal().basis().chronos_scope() == self.0 && current.chronos_scope() == self.0
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mut args = std::env::args_os().skip(1);
        let first = args.next().ok_or("configuration path required")?;
        let (observe, config_path) = if first == "--observe" {
            (true, args.next().ok_or("configuration path required")?)
        } else {
            (false, first)
        };
        if args.next().is_some() {
            return Err("usage: chronos-product-alignment-local [--observe] CONFIG.json".into());
        }
        let config = load(Path::new(&config_path))?;
        if observe {
            return observe_alignment(&config);
        }
        let timeout = Duration::from_millis(config.timeout_ms);
        let token = read_token(&config.tellus_token_file)?;
        let endpoint: SocketAddr = config
            .tellus_endpoint
            .parse()
            .map_err(|_| "invalid Tellus endpoint")?;

        let cohort = AlignmentCohortV1::new(
            principal(&config.external_advocate.principal)?,
            principal(&config.product_manager.principal)?,
            principal(&config.frontier_supervisor.principal)?,
        )
        .map_err(|_| "invalid alignment cohort")?;
        let goal_source = observe_local_vellum_artifact_v1(
            &config.vellum_socket,
            &config.vellum_credential,
            timeout,
            config.goal.locator.clone(),
            config.goal.namespace.clone(),
            config.goal.identity.clone(),
        )
        .map_err(|error| format!("Vellum goal artifact unavailable: {error:?}"))?;
        let proposal_source = observe_local_vellum_artifact_v1(
            &config.vellum_socket,
            &config.vellum_credential,
            timeout,
            config.proposal.locator.clone(),
            config.proposal.namespace.clone(),
            config.proposal.identity.clone(),
        )
        .map_err(|error| format!("Vellum proposal artifact unavailable: {error:?}"))?;
        let bootstrap = bootstrap_basis(&config, cohort.clone(), goal_source)?;
        let current = sample_local_product_alignment_v1(
            bootstrap,
            binding(&config)?,
            connections(&config, endpoint, token.clone(), timeout)?,
            LIMITS,
        )
        .map_err(|error| format!("current product-alignment frontier unavailable: {error:?}"))?;
        let proposal = GroomingProposalV1::new(
            current,
            proposal_source,
            principal(&config.product_manager.principal)?,
            GroomingRecommendationV1::EscalateHuman,
            Vec::new(),
            Vec::new(),
            LIMITS,
        )
        .map_err(|_| "product-manager proposal invalid")?;
        let journal = DirectoryAlignmentJournalV1::open(&config.register_directory, LIMITS, 16)
            .map_err(|_| "alignment register unavailable")?;
        let register = match AlignmentRegisterV1::open(journal, LIMITS) {
            Ok(register) if register.record().proposal() == &proposal => register,
            Ok(_) => {
                return Err("retained alignment proposal conflicts with current activation".into());
            }
            Err(AlignmentRegisterErrorV1::Unavailable) => {
                let journal =
                    DirectoryAlignmentJournalV1::open(&config.register_directory, LIMITS, 16)
                        .map_err(|_| "alignment register unavailable")?;
                AlignmentRegisterV1::create(journal, proposal, LIMITS)
                    .map_err(|_| "alignment proposal creation refused")?
            }
            Err(_) => return Err("retained alignment proposal unavailable".into()),
        };
        let mut runtime = LocalProductAlignmentRuntimeV1::bind(
            register,
            binding(&config)?,
            connections(&config, endpoint, token, timeout)?,
            ExactScopeDisclosure(config.chronos_scope.clone()),
            &config.listener_directory,
            16 * 1024,
            256 * 1024,
            LIMITS,
        )
        .map_err(|_| "alignment runtime binding refused")?;
        eprintln!("chronos-product-alignment-local ready");
        runtime
            .serve_one()
            .map_err(|error| format!("alignment listener stopped: {error}"))?;
        Ok(())
    }

    fn observe_alignment(config: &ConfigV1) -> Result<(), Box<dyn std::error::Error>> {
        let mut client = chronos_local::alignment::AlignmentLocalClient::new(
            &config.listener_directory,
            public_limits(),
        );
        let response = client
            .request_alignment_v1(public::Request {
                version: 1,
                command: public::Command::Observe {
                    chronos_scope: config.chronos_scope.clone(),
                },
            })
            .map_err(|_| "alignment observation unavailable")?;
        println!("{}", serde_json::to_string(&response)?);
        Ok(())
    }

    fn public_limits() -> public::Limits {
        public::Limits {
            max_field_bytes: LIMITS.max_field_bytes,
            max_evidence: LIMITS.max_evidence,
            max_next_tasks: LIMITS.max_next_tasks,
        }
    }

    fn load(path: &Path) -> Result<ConfigV1, Box<dyn std::error::Error>> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_CONFIG_BYTES {
            return Err(
                "configuration must be a nonempty regular file no larger than 64 KiB".into(),
            );
        }
        let config: ConfigV1 = serde_json::from_slice(&fs::read(path)?)?;
        if config.version != 1
            || config.timeout_ms == 0
            || config.max_moira_frame_bytes == 0
            || [
                config.chronos_directory.as_path(),
                config.moira_socket.as_path(),
                config.vellum_socket.as_path(),
                config.vellum_credential.as_path(),
                config.tellus_token_file.as_path(),
                config.register_directory.as_path(),
                config.listener_directory.as_path(),
            ]
            .iter()
            .any(|path| !path.is_absolute())
        {
            return Err("invalid product-alignment configuration".into());
        }
        Ok(config)
    }

    fn read_token(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_TOKEN_BYTES {
            return Err("Tellus token file invalid".into());
        }
        let token = fs::read_to_string(path)?;
        if token.contains(['\r', '\n']) {
            return Err("Tellus token file invalid".into());
        }
        Ok(token)
    }

    fn principal(value: &str) -> Result<PrincipalRefV1, Box<dyn std::error::Error>> {
        PrincipalRefV1::new(value.to_owned(), LIMITS)
            .map_err(|_| "invalid alignment principal".into())
    }

    fn bootstrap_basis(
        config: &ConfigV1,
        cohort: AlignmentCohortV1,
        goal: SourceReferenceV1,
    ) -> Result<AlignmentBasisV1, Box<dyn std::error::Error>> {
        let control_task = RevisionedTaskSnapshotV1 {
            reference: TaskRefV1::from_observation(
                config.control_task.task.as_str().into(),
                config.control_task.incarnation,
            ),
            revision: StateRevision::ZERO,
            task: TaskSnapshotV1 {
                phase: TaskPhase::Ready,
                attempt_number: None,
                dependency_count: 0,
                required_capability_count: 0,
                priority: 0,
                max_attempts: 1,
            },
        };
        Ok(AlignmentBasisV1::new(
            goal,
            config.chronos_scope.clone(),
            control_task,
            cohort,
            OwnerEvidenceRefV1::new(
                "vellum".into(),
                config.budget_policy.resource.clone(),
                "bootstrap-selector-only".into(),
                LIMITS,
            )
            .map_err(|_| "invalid budget-policy selector")?,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            LIMITS,
        )
        .map_err(|_| "invalid alignment bootstrap basis")?)
    }

    fn binding(config: &ConfigV1) -> Result<LocalAlignmentBindingV1, Box<dyn std::error::Error>> {
        Ok(LocalAlignmentBindingV1 {
            chronos_scope: config.chronos_scope.clone(),
            deliberation: DeliberationIdV1(parse_id(&config.deliberation)?),
            external_advocate: parse_id(&config.external_advocate.stakeholder)?,
            product_manager: parse_id(&config.product_manager.stakeholder)?,
            frontier_supervisor: parse_id(&config.frontier_supervisor.stakeholder)?,
            goal_locator: config.goal.locator.clone(),
            budget_policy_locator: config.budget_policy.locator.clone(),
            budget_policy_resource: config.budget_policy.resource.clone(),
        })
    }

    fn connections(
        config: &ConfigV1,
        endpoint: SocketAddr,
        token: String,
        timeout: Duration,
    ) -> Result<LocalAlignmentConnectionsV1, Box<dyn std::error::Error>> {
        Ok(LocalAlignmentConnectionsV1::new(
            config.chronos_directory.clone(),
            config.moira_socket.clone(),
            config.vellum_socket.clone(),
            config.vellum_credential.clone(),
            endpoint,
            token,
            timeout,
            config.max_moira_frame_bytes,
        )
        .map_err(|_| "invalid local owner connection configuration")?)
    }

    fn parse_id(value: &str) -> Result<[u8; 16], Box<dyn std::error::Error>> {
        if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("stakeholder and deliberation IDs must be 32 hexadecimal digits".into());
        }
        let mut id = [0_u8; 16];
        for (index, byte) in id.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)?;
        }
        Ok(id)
    }
}
