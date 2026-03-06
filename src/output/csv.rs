use std::fs;
use std::path::PathBuf;

use crate::error::{function_failed, BmiResult};

pub struct CsvStore {
    output_path: PathBuf,
    start_epoch: i64,
    interval: i64,
}

impl CsvStore {
    pub fn new(output_path: PathBuf, start_epoch: i64, interval: i64) -> Self {
        Self {
            output_path,
            start_epoch,
            interval,
        }
    }
}

impl super::DivideDataStore for CsvStore {
    fn write_location(&mut self, loc_id: &str, columns: &[(String, Vec<f64>)]) -> BmiResult<()> {
        let csv_path = self.output_path.join(format!("{}.csv", loc_id));

        let mut csv = String::from("Time Step,Time");
        for (name, _) in columns {
            csv.push(',');
            csv.push_str(name);
        }
        csv.push('\n');

        let n_steps = columns.first().map(|(_, v)| v.len()).unwrap_or(0);
        for step in 0..n_steps {
            let ts = self.start_epoch + (step as i64) * self.interval;
            csv.push_str(&format!("{},{}", step, format_epoch(ts)));
            for (_, vals) in columns {
                csv.push_str(&format!(",{:.9}", vals[step]));
            }
            csv.push('\n');
        }

        fs::write(&csv_path, csv).map_err(|e| {
            function_failed("csv_writer", format!("Failed to write {}: {}", csv_path.display(), e))
        })?;
        Ok(())
    }
}

fn format_epoch(epoch: i64) -> String {
    let secs_per_day: i64 = 86400;
    let mut remaining = epoch;
    let sec = remaining % 60;
    remaining /= 60;
    let min = remaining % 60;
    remaining /= 60;
    let hour = remaining % 24;
    let mut days = epoch / secs_per_day;

    let leap = |y: i32| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut year = 1970i32;
    loop {
        let yd = if leap(year) { 366 } else { 365 };
        if days < yd {
            break;
        }
        days -= yd;
        year += 1;
    }

    let mut month = 0u32;
    for m in 0..12 {
        let md = days_in_month[m] as i64 + if m == 1 && leap(year) { 1 } else { 0 };
        if days < md {
            month = m as u32 + 1;
            break;
        }
        days -= md;
    }
    let day = days + 1;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hour, min, sec
    )
}
