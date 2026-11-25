use super::*;
use crate::profile;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileArguments {
	action: String,
	#[serde(default)]
	procs: Vec<String>,
	#[serde(default = "listed")]
	top: usize
}

const fn listed() -> usize {
	40
}

impl Adapter {
	pub(super) fn profile(&mut self, args: ProfileArguments) -> Handled {
		match args.action.as_str() {
			"start" => {
				let procs = args.procs.iter().map(|proc| named_proc(proc)).collect();
				match self.request(&AuxRequest::Profile(aux::ProfileRequest::Start { procs }))? {
					AuxResponse::Profile(Ok(_)) => Ok(Value::Null),
					AuxResponse::Profile(Err(e)) => Err(e),
					other => Err(unexpected(other))
				}
			}
			"stop" => {
				let report = profile::stop("stopped", Some(args.top), |request| self.request(&request))?;
				Ok(serde_json::json!({
					"seconds": report.seconds,
					"instructions": report.hits,
					"procs": report.procs.len(),
					"html": profile::html(&report)
				}))
			}
			other => Err(format!("unknown profile action '{}': expected start or stop", other))
		}
	}
}
