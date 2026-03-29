use crate::core::SharedCore;
use crate::core_manager;
use crate::subscription;
use chrono::TimeZone;

pub struct Scheduler;

impl Scheduler {
    pub fn new() -> Self {
        Scheduler
    }

    pub async fn start(&mut self, core: SharedCore) {
        // 核心自动更新: 每天 4:30 AM
        let core_clone = core.clone();
        tokio::spawn(async move {
            loop {
                let now = chrono::Local::now();
                let duration_until_430 = calculate_duration_until_430(now);
                tokio::time::sleep(duration_until_430).await;

                match core_manager::auto_update_if_enabled(&core_clone).await {
                    Ok(Some(result)) => {
                        log::info!(
                            "定时核心自动更新完成: tag={}, asset={}",
                            result.tag,
                            result.asset_name
                        );
                    }
                    Ok(None) => {
                        log::debug!("核心自动更新检查: 无新版本");
                    }
                    Err(err) => {
                        log::warn!("定时核心自动更新失败: {err:#}");
                    }
                }
            }
        });

        // 订阅自动更新: 从配置读取间隔
        let core_clone = core.clone();
        let update_interval = {
            let core_guard = core.read().unwrap();
            core_guard.config.interval_minutes.max(5) // 最小5分钟间隔
        };

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(update_interval as u64 * 60));
            loop {
                interval.tick().await;
                match subscription::pull_all(&core_clone).await {
                    Ok(report) => {
                        log::info!(
                            "定时订阅更新完成: 总数={}, 更新={}, 失败={}",
                            report.total,
                            report.updated,
                            report.failed,
                        );
                    }
                    Err(err) => {
                        log::warn!("定时订阅更新失败: {err:#}");
                    }
                }
            }
        });

        log::info!(
            "调度器启动成功: 核心自动更新[每天04:30], 订阅自动更新[每{}分钟]",
            update_interval
        );
    }
}

fn calculate_duration_until_430(now: chrono::DateTime<chrono::Local>) -> std::time::Duration {
    let today = now.date_naive();
    let target_time = chrono::NaiveTime::from_hms_opt(4, 30, 0).expect("valid time");
    let target_datetime = chrono::Local
        .from_local_datetime(&today.and_time(target_time))
        .unwrap();

    if now < target_datetime {
        (target_datetime - now).to_std().unwrap()
    } else {
        let tomorrow = today.succ_opt().unwrap();
        let next_target = chrono::Local
            .from_local_datetime(&tomorrow.and_time(target_time))
            .unwrap();
        (next_target - now).to_std().unwrap()
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        log::debug!("调度器已停止");
    }
}
