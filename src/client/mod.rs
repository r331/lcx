pub mod graphql;
pub mod judge;
pub mod models;

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use wreq::header::{HeaderMap, HeaderValue, CONTENT_TYPE, COOKIE, ORIGIN, REFERER};
use wreq_util::Emulation;

use crate::config::Config;
use graphql::*;
use models::*;

const BASE_URL: &str = "https://leetcode.com";

/// HTTP client wrapping the LeetCode GraphQL and judge endpoints. Contains no
/// presentation logic so it can be reused by the TUI front-end. Cheap to clone
/// (the inner wreq client is reference-counted).
///
/// Uses `wreq`'s Chrome TLS/HTTP2 emulation rather than a plain HTTP client:
/// LeetCode's judge endpoints sit behind a Cloudflare bot challenge that keys
/// off the TLS fingerprint, and a stock client gets served the challenge page
/// (403) instead of a real response.
#[derive(Clone)]
pub struct LeetCodeClient {
    http: wreq::Client,
    csrf_token: Option<String>,
    base_url: String,
}

impl LeetCodeClient {
    /// Build a client from persisted config, wiring up auth cookies/headers.
    pub fn from_config(cfg: &Config) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static(BASE_URL));

        if let Some(cookie) = cookie_header(cfg)? {
            headers.insert(COOKIE, cookie);
        }

        let http = wreq::Client::builder()
            .emulation(Emulation::Chrome131)
            .default_headers(headers)
            // Cloudflare's bot-management on the judge endpoints escalates
            // scrutiny on a connection that's already carried an API call,
            // even with a correct browser TLS fingerprint. Opening a fresh
            // connection per request avoids that.
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .context("building HTTP client")?;

        Ok(Self {
            http,
            csrf_token: cfg.csrf_token.clone(),
            base_url: BASE_URL.to_string(),
        })
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Execute a GraphQL query and deserialize the `data` field.
    async fn graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T> {
        let body = serde_json::json!({ "query": query, "variables": variables });
        let resp = self
            .http
            .post(format!("{}/graphql", self.base_url))
            .header(CONTENT_TYPE, "application/json")
            .header(REFERER, format!("{}/", self.base_url))
            .json(&body)
            .send()
            .await
            .context("sending GraphQL request")?;

        let status = resp.status();
        let text = resp.text().await.context("reading GraphQL response")?;
        if !status.is_success() {
            bail!("GraphQL request failed ({status}): {text}");
        }

        let parsed: GqlResponse<T> =
            serde_json::from_str(&text).context("parsing GraphQL response")?;
        if let Some(errors) = parsed.errors {
            if !errors.is_empty() {
                let joined = errors
                    .into_iter()
                    .map(|e| e.message)
                    .collect::<Vec<_>>()
                    .join("; ");
                bail!("GraphQL error: {joined}");
            }
        }
        parsed
            .data
            .ok_or_else(|| anyhow!("GraphQL response contained no data"))
    }

    /// Return the logged-in username, or `None` if not signed in.
    pub async fn whoami(&self) -> Result<Option<String>> {
        let data: UserStatusData = self
            .graphql(USER_STATUS_QUERY, serde_json::json!({}))
            .await?;
        if data.user_status.is_signed_in {
            Ok(data.user_status.username)
        } else {
            Ok(None)
        }
    }

    /// Fetch the signed-in user's solve statistics, broken down by difficulty.
    pub async fn profile_stats(&self) -> Result<ProfileStats> {
        let username = self
            .whoami()
            .await?
            .ok_or_else(|| anyhow!("not signed in"))?;

        let data: UserProfileData = self
            .graphql(
                USER_PROFILE_QUERY,
                serde_json::json!({ "username": username }),
            )
            .await?;

        let total_of = |diff: &str| {
            data.all_questions_count
                .iter()
                .find(|d| d.difficulty.eq_ignore_ascii_case(diff))
                .map(|d| d.count)
                .unwrap_or(0)
        };

        let matched = data
            .matched_user
            .ok_or_else(|| anyhow!("profile '{username}' not found"))?;
        let solved_of = |diff: &str| {
            matched
                .submit_stats_global
                .ac_submission_num
                .iter()
                .find(|d| d.difficulty.eq_ignore_ascii_case(diff))
                .map(|d| d.count)
                .unwrap_or(0)
        };

        let stat = |diff: &str| DifficultyStat {
            solved: solved_of(diff),
            total: total_of(diff),
        };

        Ok(ProfileStats {
            username,
            easy: stat("Easy"),
            medium: stat("Medium"),
            hard: stat("Hard"),
            total: stat("All"),
        })
    }

    /// Fetch problems from the problemset list. `filters` is a raw GraphQL
    /// filter object (may be empty).
    pub async fn list_problems(
        &self,
        limit: i64,
        skip: i64,
        filters: serde_json::Value,
    ) -> Result<(i64, Vec<ProblemSummary>)> {
        let variables = serde_json::json!({
            "categorySlug": "",
            "limit": limit,
            "skip": skip,
            "filters": filters,
        });
        let data: ProblemListData = self.graphql(PROBLEM_LIST_QUERY, variables).await?;
        let list = data
            .list
            .ok_or_else(|| anyhow!("empty problem list response"))?;
        let total = list.total;
        let problems = list
            .questions
            .into_iter()
            .map(convert_summary)
            .collect::<Result<Vec<_>>>()?;
        Ok((total, problems))
    }

    /// Fetch full detail for a problem by its title slug.
    pub async fn problem_detail(&self, slug: &str) -> Result<ProblemDetail> {
        let variables = serde_json::json!({ "titleSlug": slug });
        let data: ProblemDetailData = self.graphql(PROBLEM_DETAIL_QUERY, variables).await?;
        let raw = data
            .question
            .ok_or_else(|| anyhow!("problem '{slug}' not found"))?;
        convert_detail(raw)
    }

    /// Fetch today's daily challenge (frontend id, slug, title, difficulty).
    pub async fn daily(&self) -> Result<(String, DailyChallenge)> {
        let data: DailyData = self.graphql(DAILY_QUERY, serde_json::json!({})).await?;
        let daily = data
            .daily
            .ok_or_else(|| anyhow!("no daily challenge available"))?;
        Ok((daily.date.clone(), daily))
    }

    pub(crate) async fn post_json<B: Serialize>(
        &self,
        url: &str,
        referer: &str,
        body: &B,
    ) -> Result<wreq::Response> {
        let mut req = self
            .http
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .header(REFERER, referer)
            .json(body);
        if let Some(csrf) = &self.csrf_token {
            req = req.header("x-csrftoken", csrf);
        }
        req.send().await.context("sending judge request")
    }

    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        referer: &str,
    ) -> Result<T> {
        let resp = self
            .http
            .get(url)
            .header(REFERER, referer)
            .send()
            .await
            .context("polling judge result")?;
        let status = resp.status();
        let text = resp.text().await.context("reading judge result")?;
        if !status.is_success() {
            if status == wreq::StatusCode::FORBIDDEN
                || status == wreq::StatusCode::UNAUTHORIZED
            {
                bail!("judge request rejected ({status}); your session may have expired. Run `lcx login` again.");
            }
            bail!("judge request failed ({status}): {text}");
        }
        serde_json::from_str(&text).with_context(|| format!("parsing judge result: {text}"))
    }
}

fn cookie_header(cfg: &Config) -> Result<Option<HeaderValue>> {
    let mut cookies = Vec::new();
    if let Some(session) = cfg.session.as_deref().filter(|value| !value.is_empty()) {
        cookies.push(format!("LEETCODE_SESSION={session}"));
    }
    if let Some(csrf) = cfg.csrf_token.as_deref().filter(|value| !value.is_empty()) {
        cookies.push(format!("csrftoken={csrf}"));
    }
    if let Some(clearance) = cfg.cf_clearance.as_deref().filter(|value| !value.is_empty()) {
        cookies.push(format!("cf_clearance={clearance}"));
    }
    if cookies.is_empty() {
        Ok(None)
    } else {
        Ok(Some(HeaderValue::from_str(&cookies.join("; "))?))
    }
}

#[cfg(test)]
mod tests {
    use super::cookie_header;
    use crate::config::Config;

    #[test]
    fn includes_optional_clearance_in_cookie_header() {
        let mut cfg = Config::default();
        cfg.session = Some("session-example".to_string());
        cfg.csrf_token = Some("csrf-example".to_string());
        cfg.cf_clearance = Some("clearance-example".to_string());
        let cookie = cookie_header(&cfg).unwrap().unwrap();
        assert_eq!(
            cookie.to_str().unwrap(),
            "LEETCODE_SESSION=session-example; csrftoken=csrf-example; cf_clearance=clearance-example"
        );
    }
}

fn convert_summary(raw: RawProblemSummary) -> Result<ProblemSummary> {
    Ok(ProblemSummary {
        question_id: parse_question_id(&raw.question_id, &raw.title_slug)?,
        frontend_id: raw.question_frontend_id,
        title: raw.title,
        slug: raw.title_slug,
        difficulty: raw.difficulty,
        paid_only: raw.is_paid_only,
        ac_rate: raw.ac_rate,
        status: raw.status,
        tags: raw.topic_tags.into_iter().map(|t| t.slug).collect(),
    })
}

fn convert_detail(raw: RawProblemDetail) -> Result<ProblemDetail> {
    Ok(ProblemDetail {
        question_id: parse_question_id(&raw.question_id, &raw.title_slug)?,
        frontend_id: raw.question_frontend_id,
        title: raw.title,
        slug: raw.title_slug,
        difficulty: raw.difficulty,
        content: raw.content.unwrap_or_default(),
        code_snippets: raw
            .code_snippets
            .unwrap_or_default()
            .into_iter()
            .map(|s| CodeSnippet {
                lang: s.lang,
                lang_slug: s.lang_slug,
                code: s.code,
            })
            .collect(),
        sample_test_case: raw.sample_test_case.unwrap_or_default(),
        example_testcases: raw.example_testcases.unwrap_or_default(),
    })
}

/// Parse LeetCode's string `questionId` into the numeric id used for the judge
/// endpoints, failing loudly rather than silently defaulting to 0 (which would
/// otherwise be sent as the id on test/submit).
fn parse_question_id(raw: &str, slug: &str) -> Result<i64> {
    raw.parse()
        .with_context(|| format!("unexpected question id '{raw}' for problem '{slug}'"))
}
