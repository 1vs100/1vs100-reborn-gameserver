use std::{
    io::{self, Write},
    time::{Duration, SystemTime},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use flate2::{Compression, write::GzEncoder};
use rsa::{
    RsaPrivateKey,
    pkcs1v15::SigningKey,
    signature::{SignatureEncoding, Signer},
};
use sha1::Sha1;

use crate::{
    schedule::{Schedule, Showing},
    time::Utc,
};

/// Pack content ids in Grid `<Packages>`; override per locale/episode via `GAMESERVER_*_CONTENT_ID`.
const DEFAULT_LAUNCH_CONTENT_ID: &str = "988237803712CB87513237A914D367C6AAEE2410";
const DEFAULT_SHOW_CONTENT_ID: &str = "77205BC1241C708A8A10B7303AF90E5EDD0286B1";
const DEFAULT_EPISODE_CONTENT_ID: &str = "83136FF343124795DC1806A2DB044476589BD5C0";

/// Grid settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridConfig {
    /// Upper-case, 40 hex digits each.
    pub launch_content_id: String,
    pub show_content_id: String,
    pub episode_content_id: String,
}

impl Default for GridConfig {
    fn default() -> Self {
        Self {
            launch_content_id: DEFAULT_LAUNCH_CONTENT_ID.into(),
            show_content_id: DEFAULT_SHOW_CONTENT_ID.into(),
            episode_content_id: DEFAULT_EPISODE_CONTENT_ID.into(),
        }
    }
}

/// One Grid sessions reply, build-independent.
struct Listing {
    start: String,
    end: String,
    group_id: String,
    session_id: String,
    episode_guid: String,
    channel: String,
}

/// `EpisodeGuid` without a schedule.
const UNSCHEDULED_EPISODE_GUID: &str = "00000000-0000-0000-0000-000000000002";

fn listing(channel: &str, showing: &Showing, now: SystemTime) -> Listing {
    let (start, group_id, episode_guid) = match showing {
        // The title drops listed sessions already started, so a running show lists as starting imminently.
        Showing::Scheduled(show) => (
            SystemTime::from(show.plan.start).max(now + RUNNING_SHOW_LISTED_START_LEAD),
            crate::schedule::season_id(show.plan.start),
            show.episode_id.clone(),
        ),
        // Without a schedule: the canonical episode as the episode group (1149 hands Lua the
        // group id as the episode id, which keys AvailableLanguages).
        Showing::Unscheduled { start, .. } => (
            *start,
            crate::schedule::CANONICAL_EPISODE.to_owned(),
            UNSCHEDULED_EPISODE_GUID.to_owned(),
        ),
    };
    Listing {
        start: Utc::from(start).to_grid(),
        end: Utc::from(showing.end()).to_grid(),
        group_id,
        session_id: showing.session_id(),
        episode_guid,
        channel: xml_escape(channel),
    }
}

/// v2.0.1317.0 Grid sessions document.
pub fn sessions_xml(
    config: &GridConfig,
    schedule: &mut Schedule,
    channel: &str,
    date: &str,
    now: SystemTime,
) -> String {
    let GridConfig {
        launch_content_id: launch_id,
        show_content_id: show_id,
        episode_content_id: episode_id,
    } = config;
    let Listing {
        start,
        end,
        group_id,
        session_id,
        episode_guid,
        channel,
    } = {
        let showing = schedule.showing(now, date_offset(date, now));
        showing.log_handed_out("grid", now, None);
        listing(channel, &showing, now)
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<Sessions>\n\
  <Session>\n\
    <XboxTitleId>584A07D1</XboxTitleId>\n\
    <EpisodeGroupId>{group_id}</EpisodeGroupId>\n\
    <StartTimeUtc>{start}</StartTimeUtc>\n\
    <EndTimeUtc>{end}</EndTimeUtc>\n\
    <SessionId>{session_id}</SessionId>\n\
    <EpisodeId>{episode_guid}</EpisodeId>\n\
    <ChannelName>{channel}</ChannelName>\n\
    <EpisodeGroupName>OneVsLive</EpisodeGroupName>\n\
    <NameShort>1 vs 100</NameShort>\n\
    <NameFull>1 vs 100 Live</NameFull>\n\
    <TitleSponsor/>\n\
    <BusinessModel/>\n\
    <Packages>\n\
      <Package>\n\
        <MarketplaceOfferId>00000000000000000000000000000000</MarketplaceOfferId>\n\
        <MarketplaceMediaId>00000000000000000000000000000000</MarketplaceMediaId>\n\
        <MarketplaceMediaInstanceId>00000000000000000000000000000000</MarketplaceMediaInstanceId>\n\
        <InstallSizeInKB>0</InstallSizeInKB>\n\
        <Priority>0</Priority>\n\
        <MarketplaceMediaType>0</MarketplaceMediaType>\n\
        <MarketplaceContentId>{launch_id}</MarketplaceContentId>\n\
        <Name>1 vs 100 Launch</Name>\n\
      </Package>\n\
      <Package>\n\
        <MarketplaceOfferId>00000000000000000000000000000000</MarketplaceOfferId>\n\
        <MarketplaceMediaId>00000000000000000000000000000000</MarketplaceMediaId>\n\
        <MarketplaceMediaInstanceId>00000000000000000000000000000000</MarketplaceMediaInstanceId>\n\
        <InstallSizeInKB>0</InstallSizeInKB>\n\
        <Priority>2</Priority>\n\
        <MarketplaceMediaType>0</MarketplaceMediaType>\n\
        <MarketplaceContentId>{show_id}</MarketplaceContentId>\n\
        <Name>1 vs 100 Show</Name>\n\
      </Package>\n\
      <Package>\n\
        <MarketplaceOfferId>00000000000000000000000000000000</MarketplaceOfferId>\n\
        <MarketplaceMediaId>00000000000000000000000000000000</MarketplaceMediaId>\n\
        <MarketplaceMediaInstanceId>00000000000000000000000000000000</MarketplaceMediaInstanceId>\n\
        <InstallSizeInKB>0</InstallSizeInKB>\n\
        <Priority>2</Priority>\n\
        <MarketplaceMediaType>0</MarketplaceMediaType>\n\
        <MarketplaceContentId>{episode_id}</MarketplaceContentId>\n\
        <Name>1 vs 100 Episode</Name>\n\
      </Package>\n\
    </Packages>\n\
  </Session>\n\
</Sessions>\n"
    )
}

/// Listed show `offset` slots ahead and the one after.
fn current_and_next(
    schedule: &mut Schedule,
    channel: &str,
    offset: usize,
    now: SystemTime,
) -> [(Listing, Showing); 2] {
    schedule
        .showing_pair(now, offset)
        .map(|showing| (listing(channel, &showing, now), showing))
}

/// Listed show.
pub struct ServedShow {
    pub session_id: String,
    pub start_utc: String,
    pub end_utc: String,
    pub mode: crate::schedule::Mode,
}

/// Current and next show of today's Grid fetch.
pub fn served_window(schedule: &mut Schedule, now: SystemTime) -> [ServedShow; 2] {
    current_and_next(schedule, "1vs100", 0, now).map(|(listing, showing)| ServedShow {
        session_id: listing.session_id,
        start_utc: listing.start,
        end_utc: listing.end,
        mode: showing.mode(),
    })
}

/// v2.0.1149.0 Grid sessions document. Lists the requested slot and the next: 1149 Next skips the first unended session.
pub fn sessions_xml_1149(
    config: &GridConfig,
    schedule: &mut Schedule,
    channel: &str,
    date: &str,
    now: SystemTime,
    host: &str,
) -> String {
    let [(current, showing), (next, _)] =
        current_and_next(schedule, channel, date_offset(date, now), now);
    showing.log_handed_out("grid", now, None);
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<Sessions>\n\
  <ChannelName>{}</ChannelName>\n\
{}{}</Sessions>\n",
        current.channel,
        session_xml_1149(config, &current, host),
        session_xml_1149(config, &next, host)
    )
}

fn session_xml_1149(config: &GridConfig, listing: &Listing, host: &str) -> String {
    /// The Marketplace ids of a listing without a Marketplace offer: the nil GUID.
    const ZERO_GUID: &str = "00000000-0000-0000-0000-000000000000";
    let Listing {
        start,
        end,
        group_id,
        session_id,
        episode_guid,
        channel,
    } = listing;
    let host = xml_escape(host);
    let package = |priority: u8, content_id: &str| {
        format!(
            "      <Package>\n\
        <MarketplaceOfferId>{ZERO_GUID}</MarketplaceOfferId>\n\
        <MarketplaceMediaId>{ZERO_GUID}</MarketplaceMediaId>\n\
        <MarketplaceMediaInstanceId>{ZERO_GUID}</MarketplaceMediaInstanceId>\n\
        <InstallSizeInKB>0</InstallSizeInKB>\n\
        <Priority>{priority}</Priority>\n\
        <MarketplaceMediaType>18</MarketplaceMediaType>\n\
        <MarketplaceContentId>{content_id}</MarketplaceContentId>\n\
      </Package>\n"
        )
    };
    let packages = [
        package(0, &config.launch_content_id),
        package(2, &config.show_content_id),
        package(2, &config.episode_content_id),
    ]
    .concat();
    format!(
        "  <Session>\n\
    <ChannelName>{channel}</ChannelName>\n\
    <SessionId>{session_id}</SessionId>\n\
    <TitleType>1</TitleType>\n\
    <EpisodeGroupId>{group_id}</EpisodeGroupId>\n\
    <EpisodeId>{episode_guid}</EpisodeId>\n\
    <XboxTitleId>584A07D1</XboxTitleId>\n\
    <PreviewUrl>http://{host}/grid/v1/media/preview</PreviewUrl>\n\
    <MediaUrl>http://{host}/grid/v1/media/media</MediaUrl>\n\
    <ThumbnailUrl>http://{host}/grid/v1/media/thumbnail</ThumbnailUrl>\n\
    <HasRewards>False</HasRewards>\n\
    <IsFeaturedItem>False</IsFeaturedItem>\n\
    <IsNew>False</IsNew>\n\
    <StartTimeUtc>{start}</StartTimeUtc>\n\
    <EndTimeUtc>{end}</EndTimeUtc>\n\
    <NameShort>1 vs 100</NameShort>\n\
    <EpisodeGroupName>OneVsLive</EpisodeGroupName>\n\
    <IsPublished>True</IsPublished>\n\
    <Packages>\n\
{packages}\
    </Packages>\n\
  </Session>\n"
    )
}

pub fn signed_response(
    path: &str,
    host: &str,
    xml: &[u8],
    now: SystemTime,
    key: &RsaPrivateKey,
) -> io::Result<Vec<u8>> {
    let expires = Utc::from(now + Duration::from_secs(6 * 60 * 60)).to_rfc1123();
    let urn = format!("{host}{path}");
    let mut canonical = Vec::new();
    write!(
        canonical,
        "X-HTTPSig-Expires: {expires}\r\nX-HTTPSig-Urn: {urn}\r\n\r\n"
    )?;
    canonical.extend_from_slice(xml);
    let signature = SigningKey::<Sha1>::new(key.clone()).sign(&canonical);
    let signature = STANDARD.encode(signature.to_bytes());

    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(xml)?;
    let body = encoder.finish()?;

    let headers = format!(
        "HTTP/1.1 200 OK\r\n\
Content-Type: text/xml; charset=utf-8\r\n\
Content-Encoding: gzip\r\n\
Content-Length: {}\r\n\
X-HTTPSig-Expires: {expires}\r\n\
X-HTTPSig-Urn: {urn}\r\n\
X-HTTPSig-Signature: {signature}\r\n\
Connection: Keep-alive\r\n\r\n",
        body.len()
    );
    let mut response = headers.into_bytes();
    response.extend_from_slice(&body);
    Ok(response)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Listed start lead for a running show.
const RUNNING_SHOW_LISTED_START_LEAD: Duration = Duration::from_secs(30);

/// Request `YYYYMMDD` as days past today, clamped to 0..=2.
fn date_offset(date: &str, now: SystemTime) -> usize {
    let parse = |d: &str| -> Option<i64> {
        if d.len() != 8 || !d.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let day = Utc::from_date(
            d[..4].parse().ok()?,
            d[4..6].parse().ok()?,
            d[6..].parse().ok()?,
        )?;
        Some(day.unix_days())
    };
    let today = Utc::from(now).unix_days();
    parse(date).map_or(0, |d| (d - today).clamp(0, 2) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use flate2::read::GzDecoder;
    use rsa::{
        RsaPublicKey,
        pkcs1::DecodeRsaPrivateKey,
        pkcs1v15::{Signature, VerifyingKey},
        signature::Verifier,
    };
    use sha1::Sha1;
    use std::{io::Read, time::Duration};

    fn key() -> RsaPrivateKey {
        RsaPrivateKey::from_pkcs1_pem(include_str!("../keys/grid_lan_private.pem")).unwrap()
    }

    fn fixed_now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_510_400)
    }

    fn split_response(response: &[u8]) -> (&str, &[u8]) {
        let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (
            std::str::from_utf8(&response[..split]).unwrap(),
            &response[split + 4..],
        )
    }

    fn header<'a>(headers: &'a str, name: &str) -> &'a str {
        headers
            .lines()
            .find_map(|line| line.strip_prefix(name).map(str::trim))
            .unwrap()
    }

    /// A Grid document as its elements, one per line.
    fn lines(xml: String) -> String {
        xml.replace("><", ">\n<")
    }

    /// The sessions reply of each build, over the fresh schedule.
    #[test]
    fn the_sessions_replies() {
        let now = fixed_now();
        let reply = lines(sessions_xml(
            &GridConfig::default(),
            &mut Schedule::default(),
            "1vs100",
            "20260927",
            now,
        ));
        let retail = lines(sessions_xml_1149(
            &GridConfig::default(),
            &mut Schedule::default(),
            "1vs100",
            "20260927",
            now,
            "192.168.42.1",
        ));
        insta::assert_snapshot!(format!("1317:\n{reply}\n1149:\n{retail}"));
    }

    #[test]
    fn signed_response_gzips_xml_and_signs_canonical_uncompressed_bytes() {
        let now = fixed_now();
        let mut schedule = Schedule::default();
        let xml = sessions_xml(
            &GridConfig::default(),
            &mut schedule,
            "1vs100",
            "20260927",
            now,
        );
        let response = signed_response(
            "/grid/v1/sessions/1vs100/en-US/20260927",
            "192.0.2.10",
            xml.as_bytes(),
            now,
            &key(),
        )
        .unwrap();
        let (headers, body) = split_response(&response);

        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(headers.contains("\r\nContent-Type: text/xml; charset=utf-8\r\n"));
        assert!(headers.contains("\r\nContent-Encoding: gzip\r\n"));
        assert_eq!(header(headers, "Content-Length:"), body.len().to_string());
        assert_eq!(
            header(headers, "X-HTTPSig-Expires:"),
            "Sun, 27 Sep 2026 18:00:00 GMT"
        );
        assert_eq!(
            header(headers, "X-HTTPSig-Urn:"),
            "192.0.2.10/grid/v1/sessions/1vs100/en-US/20260927"
        );

        let mut inflated = Vec::new();
        GzDecoder::new(body).read_to_end(&mut inflated).unwrap();
        assert_eq!(inflated, xml.as_bytes());

        let signature_bytes =
            base64::Engine::decode(&STANDARD, header(headers, "X-HTTPSig-Signature:")).unwrap();
        assert_eq!(signature_bytes.len(), 128);
        let signature = Signature::try_from(signature_bytes.as_slice()).unwrap();
        let canonical = format!(
            "X-HTTPSig-Expires: {}\r\nX-HTTPSig-Urn: {}\r\n\r\n{}",
            header(headers, "X-HTTPSig-Expires:"),
            header(headers, "X-HTTPSig-Urn:"),
            xml
        );
        let verifier = VerifyingKey::<Sha1>::new(RsaPublicKey::from(&key()));
        verifier.verify(canonical.as_bytes(), &signature).unwrap();

        let tampered = canonical.replace("1 vs 100 Live", "1 vs 100 Evil");
        assert!(verifier.verify(tampered.as_bytes(), &signature).is_err());
    }

    #[test]
    fn date_offsets_follow_the_requested_day() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_510_400); // 2026-09-27
        assert_eq!(date_offset("20260927", now), 0);
        assert_eq!(date_offset("20260928", now), 1);
        assert_eq!(date_offset("20260929", now), 2);
        assert_eq!(date_offset("20260101", now), 0);
        assert_eq!(date_offset("garbage", now), 0);
    }

    #[test]
    fn scheduled_shows_drive_the_grid_window_and_ids() {
        let storage = std::sync::Arc::new(crate::storage::MemoryStorage::default());
        crate::storage::Storage::set_programme(
            storage.as_ref(),
            &crate::programme::Programme::default(),
        )
        .unwrap();
        let mut schedule =
            Schedule::load(crate::schedule::ScheduleConfig::default(), storage).unwrap();
        let now = SystemTime::now();
        let today = Utc::from(now).to_grid();
        let date = format!("{}{}{}", &today[..4], &today[5..7], &today[8..10]);
        let current = sessions_xml(&GridConfig::default(), &mut schedule, "1vs100", &date, now);
        let crate::schedule::Showing::Scheduled(first) = schedule.showing(now, 0) else {
            panic!("a programme serves scheduled shows");
        };
        assert!(current.contains(&format!("<SessionId>{}</SessionId>", first.session_id)));
        assert!(current.contains(&format!("<EpisodeId>{}</EpisodeId>", first.episode_id)));
        assert!(current.contains(&format!(
            "<EpisodeGroupId>{}</EpisodeGroupId>",
            crate::schedule::season_id(first.plan.start)
        )));
        assert!(current.contains(&format!(
            "<EndTimeUtc>{}</EndTimeUtc>",
            first.plan.end.to_grid()
        )));
        // a running show is listed as starting imminently so the title takes it as Current
        assert!(current.contains(&format!(
            "<StartTimeUtc>{}</StartTimeUtc>",
            Utc::from(now + RUNNING_SHOW_LISTED_START_LEAD).to_grid()
        )));
        let tomorrow = Utc::from(now + Duration::from_secs(86_400)).to_grid();
        let next_date = format!("{}{}{}", &tomorrow[..4], &tomorrow[5..7], &tomorrow[8..10]);
        let next = schedule.showing(now, 1).session_id();
        let reply = sessions_xml(
            &GridConfig::default(),
            &mut schedule,
            "1vs100",
            &next_date,
            now,
        );
        assert!(reply.contains(&format!("<SessionId>{next}</SessionId>")));
        assert_ne!(first.session_id, next);
        // without a programme the constant ids come back
        assert!(
            sessions_xml(
                &GridConfig::default(),
                &mut Schedule::default(),
                "1vs100",
                &date,
                now
            )
            .contains("<SessionId>00000000-0000-0000-0000-000000000001</SessionId>")
        );
    }
}
