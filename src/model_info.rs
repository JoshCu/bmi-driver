use std::path::Path;

use crate::error::BmiResult;
use crate::runner::ModelRunner;

/// Print all unit conversion info for the first location, then return.
pub fn print_units(realization: &Path, locations: &[String]) -> BmiResult<()> {
    let mut runner = ModelRunner::from_config(realization)?;
    if let Some(loc) = locations.first() {
        runner.initialize(loc)?;
        print_unit_conversions(&runner, false);
        runner.finalize()?;
    } else {
        eprintln!("No locations found.");
    }
    Ok(())
}

/// Print a summary of each model's inputs and outputs, then return.
pub fn inspect_models(realization: &Path, locations: &[String]) -> BmiResult<()> {
    let mut runner = ModelRunner::from_config(realization)?;
    let loc = match locations.first() {
        Some(l) => l,
        None => {
            eprintln!("No locations found.");
            return Ok(());
        }
    };
    runner.initialize(loc)?;

    let modules: Vec<_> = runner.config.modules().into_iter().cloned().collect();

    eprintln!("Realization: {} model(s)\n", runner.models.len());

    for (i, model) in runner.models.iter().enumerate() {
        let module = modules.get(i);
        eprintln!("━━━ Model {}: {} ━━━", i + 1, model.name);

        if let Some(m) = module {
            if !m.params.library_file.is_empty() {
                eprintln!("  library: {}", m.params.library_file);
            }
            if !m.params.init_config.is_empty() {
                eprintln!("  init_config: {}", m.params.init_config);
            }
        }

        eprintln!(
            "  timestep: {}s ({} steps)",
            model.timestep_info.dt_seconds, model.timestep_info.num_steps
        );

        // Inputs
        let input_names = model.model.get_input_var_names().unwrap_or_default();
        eprintln!("\n  Inputs ({}):", input_names.len());
        for name in &input_names {
            let units = model.model.get_var_units(name).unwrap_or_default();
            let units_str = if units.is_empty() { "?" } else { &units };

            if let Some(source) = model.input_map.get(name) {
                let source_label = runner.source_label(source);
                let conv_str = model
                    .input_conversions
                    .get(name)
                    .filter(|c| !c.is_identity())
                    .map(|c| format!(" [{}]", c))
                    .unwrap_or_default();
                eprintln!(
                    "    {} [{}] ← {} ({}){conv_str}",
                    name, units_str, source, source_label
                );
            } else {
                eprintln!("    {} [{}]  (unmapped)", name, units_str);
            }
        }

        // Outputs
        let output_names = model.model.get_output_var_names().unwrap_or_default();
        eprintln!("\n  Outputs ({}):", output_names.len());
        for name in &output_names {
            let units = model.model.get_var_units(name).unwrap_or_default();
            let units_str = if units.is_empty() { "?" } else { &units };
            let is_main = name == &model.main_output;
            let tag = if is_main { " (main)" } else { "" };
            eprintln!("    {} [{}]{}", name, units_str, tag);
        }

        eprintln!();
    }

    runner.finalize()?;
    Ok(())
}

/// Print unit conversions to stderr.
/// If `active_only` is true, only prints non-identity conversions.
/// If false, prints all variable mappings including those without unit info.
pub fn print_unit_conversions(runner: &ModelRunner, active_only: bool) {
    if !active_only {
        eprintln!("Unit conversions for this run:");
    }
    let mut any = false;
    for m in &runner.models {
        for (model_input, source_var) in &m.input_map {
            let source_label = runner.source_label(source_var);
            if let Some(conv) = m.input_conversions.get(model_input) {
                if active_only && conv.is_identity() {
                    continue;
                }
                eprintln!(
                    "  {}: {} ← {} ({}): {}",
                    m.name, model_input, source_var, source_label, conv
                );
            } else if !active_only {
                eprintln!(
                    "  {}: {} ← {} ({}): no unit info available",
                    m.name, model_input, source_var, source_label
                );
            } else {
                continue;
            }
            any = true;
        }
    }
    if active_only && any {
        eprintln!();
    }
    if !active_only && !any {
        eprintln!("  (no variable mappings)");
    }
}
