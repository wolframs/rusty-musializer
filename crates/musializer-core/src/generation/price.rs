//! What a clip will cost, before it is asked for, and what shape it is asked
//! in.
//!
//! The SKUs mix units (read 2026-11-14): `cents_per_second_output[_<tier>]` is
//! cents per second; `duration_seconds[_without_audio|_with_audio][_<tier>]`
//! dollars per second; `video_tokens[...]` dollars per token, a token being
//! width × height × fps / 1024 at 24 fps by ByteDance's published formula,
//! which OpenRouter does not state — such a price is marked approximate. A SKU
//! for the asked tier wins over the unsuffixed one, which on the list prices
//! the model's other tiers (Veo 3.1 Lite: `_720p` 0.03, unsuffixed 0.05 for
//! 1080p). A request never asks for sound, so a `_without_audio` SKU wins over
//! a plain one, and a `_with_audio` one is used only when nothing else prices
//! the tier: an over-estimate, never an under-estimate.

use super::catalog::{Catalog, Offer};

/// Why there is no number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnknownPrice {
    /// No price list has been read on this machine.
    NoList,
    /// The list is older than [`super::catalog::STALE_AFTER_DAYS`].
    Stale { age_days: i64, date: String },
    /// The model is not on the list (any more).
    NotListed,
    /// The list has the model but no SKU this module can read for the tier.
    NoRate { resolution: String },
}

impl UnknownPrice {
    /// The sentence a reader gets, beginning with the word "unknown".
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::NoList => "price unknown: no price list has been read yet".to_owned(),
            Self::Stale { age_days, date } => {
                format!("price unknown: the price list is {age_days} days old ({date})")
            }
            Self::NotListed => "price unknown: the model is not on the price list".to_owned(),
            Self::NoRate { resolution } => {
                format!(
                    "price unknown: the list gives no rate this application reads for {resolution}"
                )
            }
        }
    }
}

/// A rate per second of clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Rate {
    pub usd_per_second: f64,
    /// Derived from a token price by a formula OpenRouter does not state.
    pub approximate: bool,
    /// The SKU it came from, for the record.
    pub sku: String,
}

/// An estimate in dollars.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub usd: f64,
    pub rate: Rate,
    pub seconds: u32,
    /// What the list may add per frame image (`reference_images`, Hailuo 3:
    /// whether frame images count as references is not documented, so it is
    /// counted — an over-estimate rather than a surprise).
    pub per_image_usd: f64,
}

impl Estimate {
    /// `$0.40`, or `~$0.40` when approximate. Two decimals, rounded up to the
    /// cent: a button that says forty cents never spends forty-one.
    #[must_use]
    pub fn dollars(&self) -> String {
        format!(
            "{}${:.2}",
            if self.rate.approximate { "~" } else { "" },
            ceil_cents(self.usd)
        )
    }
}

/// Dollars rounded up to whole cents (tolerating float noise below a
/// thousandth of a cent).
#[must_use]
pub fn ceil_cents(usd: f64) -> f64 {
    ((usd * 100.0) - 1.0e-5).ceil().max(0.0) / 100.0
}

/// A price, or why there is none.
#[derive(Clone, Debug, PartialEq)]
pub enum Price {
    Known(Estimate),
    Unknown(UnknownPrice),
}

impl Price {
    #[must_use]
    pub fn usd(&self) -> Option<f64> {
        match self {
            Self::Known(estimate) => Some(estimate.usd),
            Self::Unknown(_) => None,
        }
    }

    /// `$0.40` or `price unknown`.
    #[must_use]
    pub fn short(&self) -> String {
        match self {
            Self::Known(estimate) => estimate.dollars(),
            Self::Unknown(_) => "price unknown".to_owned(),
        }
    }
}

fn tier_suffix(resolution: &str) -> String {
    resolution.to_ascii_lowercase()
}

/// The rate per second at `resolution` and `size`, without audio.
#[must_use]
pub fn rate(offer: &Offer, resolution: &str, size: (u32, u32)) -> Option<Rate> {
    let tier = tier_suffix(resolution);
    let find = |names: &[String]| {
        names
            .iter()
            .find_map(|name| offer.skus.get(name).map(|value| (name.clone(), *value)))
    };
    let tiered = |base: &str| vec![format!("{base}_{tier}"), base.to_owned()];
    // Each family: the tier's own SKU, then the unsuffixed one.
    let cents = [
        format!("cents_per_video_output_second_{tier}"),
        format!("cents_per_second_output_{tier}"),
        "cents_per_video_output_second".to_owned(),
        "cents_per_second_output".to_owned(),
    ];
    if let Some((sku, value)) = find(&cents) {
        return Some(Rate {
            usd_per_second: value / 100.0,
            approximate: false,
            sku,
        });
    }
    let mut dollars = tiered("duration_seconds_without_audio");
    dollars.extend(tiered("duration_seconds"));
    dollars.extend(tiered("duration_seconds_with_audio"));
    // A tier SKU of any audio kind beats an unsuffixed one: order by tier first.
    dollars.sort_by_key(|name| !name.ends_with(&format!("_{tier}")));
    if let Some((sku, value)) = find(&dollars) {
        return Some(Rate {
            usd_per_second: value,
            approximate: false,
            sku,
        });
    }
    let mut tokens = tiered("video_tokens_without_audio");
    tokens.extend(tiered("video_tokens"));
    tokens.sort_by_key(|name| !name.ends_with(&format!("_{tier}")));
    if let Some((sku, value)) = find(&tokens) {
        let per_second = f64::from(size.0) * f64::from(size.1) * 24.0 / 1024.0;
        return Some(Rate {
            usd_per_second: value * per_second,
            approximate: true,
            sku,
        });
    }
    None
}

/// The estimate for a request, from the cached list, at `now_unix`.
#[must_use]
pub fn estimate(
    catalog: Option<&Catalog>,
    now_unix: i64,
    model: &str,
    seconds: u32,
    shape: &Shape,
    frame_images: u32,
) -> Price {
    let Some(catalog) = catalog else {
        return Price::Unknown(UnknownPrice::NoList);
    };
    if catalog.is_stale(now_unix) {
        return Price::Unknown(UnknownPrice::Stale {
            age_days: catalog.age_days(now_unix),
            date: catalog.fetched_date(),
        });
    }
    let Some(offer) = catalog.offer(model) else {
        return Price::Unknown(UnknownPrice::NotListed);
    };
    let Some(rate) = rate(offer, &shape.resolution, shape.size) else {
        return Price::Unknown(UnknownPrice::NoRate {
            resolution: shape.resolution.clone(),
        });
    };
    let per_image_usd = offer.skus.get("reference_images").copied().unwrap_or(0.0);
    Price::Known(Estimate {
        usd: rate.usd_per_second * f64::from(seconds) + per_image_usd * f64::from(frame_images),
        rate,
        seconds,
        per_image_usd,
    })
}

/// The shape a clip is asked in: a resolution tier and an aspect ratio the
/// model lists, and the pixel size that implies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub resolution: String,
    pub aspect_ratio: String,
    /// What the tier and ratio make, for the token estimate and the frame
    /// images' size.
    pub size: (u32, u32),
}

/// The short side a tier stands for.
fn tier_side(resolution: &str) -> Option<u32> {
    Some(match resolution {
        "360p" => 360,
        "480p" => 480,
        "720p" => 720,
        "768p" => 768,
        "1080p" | "1K" => 1080,
        "2K" => 1440,
        "4K" => 2160,
        _ => return None,
    })
}

fn ratio(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once(':')?;
    let pair = (w.parse().ok()?, h.parse().ok()?);
    (pair.0 > 0 && pair.1 > 0).then_some(pair)
}

/// The shape the project's output asks this model for.
///
/// The aspect must be one the model lists, within 1 % of the output's own
/// (1920x1080 and 1280x720 are both 16:9); there is no cropping here. The tier
/// is the output's short side when listed, else the smallest listed tier
/// above it, else the largest below: Hailuo 3 lists only 2K, so a 1080p
/// project gets a 2K clip, which the importer fits like any other.
///
/// # Errors
///
/// The sentence for a reader: the model makes no shape the output is.
pub fn shape(offer: &Offer, output: (u32, u32)) -> Result<Shape, String> {
    let wanted = f64::from(output.0) / f64::from(output.1.max(1));
    let Some(aspect) = offer.aspect_ratios.iter().find(|text| {
        ratio(text).is_some_and(|(w, h)| (f64::from(w) / f64::from(h) / wanted - 1.0).abs() < 0.01)
    }) else {
        return Err(format!(
            "{} makes {} clips; the project's output is {}x{}",
            offer.name,
            if offer.aspect_ratios.is_empty() {
                "no listed aspect of".to_owned()
            } else {
                offer.aspect_ratios.join(", ")
            },
            output.0,
            output.1
        ));
    };
    let short = output.0.min(output.1);
    let mut tiers: Vec<(&String, u32)> = offer
        .resolutions
        .iter()
        .filter_map(|tier| Some((tier, tier_side(tier)?)))
        .collect();
    tiers.sort_by_key(|(_, side)| *side);
    let chosen = tiers
        .iter()
        .find(|(_, side)| *side >= short)
        .or_else(|| tiers.last())
        .ok_or_else(|| format!("{} lists no resolution this application knows", offer.name))?;
    let (w, h) = ratio(aspect).unwrap_or((16, 9));
    let side = chosen.1;
    // The short side is the tier; the long one follows the ratio, rounded to
    // an even number as encoders want.
    let size = if w >= h {
        (even(f64::from(side) * f64::from(w) / f64::from(h)), side)
    } else {
        (side, even(f64::from(side) * f64::from(h) / f64::from(w)))
    };
    Ok(Shape {
        resolution: chosen.0.clone(),
        aspect_ratio: aspect.clone(),
        size,
    })
}

fn even(value: f64) -> u32 {
    ((value / 2.0).round() as u32) * 2
}

#[cfg(test)]
mod tests {
    use super::super::catalog::{tests::LIVE, Catalog};
    use super::*;

    const NOW: i64 = 1_794_700_000;

    fn catalog() -> Catalog {
        Catalog::from_live(LIVE.as_bytes(), NOW).unwrap()
    }

    /// The numbers on 2026-11-14's list, pinned: Veo 3.1 Lite at 720p is
    /// 0.03 $/s and at 1080p 0.05 $/s (the unsuffixed no-audio SKU), so four
    /// seconds are 12 and 20 cents.
    #[test]
    fn the_estimate_is_the_listed_rate_times_the_seconds() {
        let catalog = catalog();
        let veo = catalog.offer("google/veo-3.1-lite").unwrap();
        let p720 = shape(veo, (1280, 720)).unwrap();
        assert_eq!(p720.resolution, "720p");
        assert_eq!(p720.aspect_ratio, "16:9");
        assert_eq!(p720.size, (1280, 720));
        let price = estimate(Some(&catalog), NOW, veo.id.as_str(), 4, &p720, 2);
        let Price::Known(known) = &price else {
            panic!("{price:?}")
        };
        assert_eq!(known.rate.sku, "duration_seconds_without_audio_720p");
        assert!((known.usd - 0.12).abs() < 1e-12, "{}", known.usd);
        assert_eq!(known.dollars(), "$0.12");
        let p1080 = shape(veo, (1920, 1080)).unwrap();
        let Price::Known(known) = estimate(Some(&catalog), NOW, veo.id.as_str(), 8, &p1080, 0)
        else {
            panic!()
        };
        assert_eq!(known.rate.sku, "duration_seconds_without_audio");
        assert!((known.usd - 0.40).abs() < 1e-12);
    }

    /// Seedance prices by token: 1920 × 1080 × 24 / 1024 = 48 600 tokens a
    /// second at the 1080p SKU, $0.37422 a second, marked approximate.
    #[test]
    fn a_token_price_is_an_approximate_rate() {
        let catalog = catalog();
        let seedance = catalog.offer("bytedance/seedance-2.0").unwrap();
        let shape = shape(seedance, (1920, 1080)).unwrap();
        let rate = rate(seedance, &shape.resolution, shape.size).unwrap();
        assert_eq!(rate.sku, "video_tokens_1080p");
        assert!(rate.approximate);
        assert!((rate.usd_per_second - 48_600.0 * 0.000_007_7).abs() < 1e-12);
        let Price::Known(known) = estimate(Some(&catalog), NOW, &seedance.id, 4, &shape, 0) else {
            panic!()
        };
        assert_eq!(known.dollars(), "~$1.50");
    }

    #[test]
    fn no_list_an_old_list_or_no_rate_is_unknown_by_name() {
        let catalog = catalog();
        let veo = catalog.offer("google/veo-3.1-lite").unwrap();
        let p720 = shape(veo, (1280, 720)).unwrap();
        let unknown = |price: Price| match price {
            Price::Unknown(reason) => reason,
            Price::Known(known) => panic!("{known:?}"),
        };
        assert_eq!(
            unknown(estimate(None, NOW, &veo.id, 4, &p720, 0)),
            UnknownPrice::NoList
        );
        let stale = unknown(estimate(
            Some(&catalog),
            NOW + 8 * 86_400,
            &veo.id,
            4,
            &p720,
            0,
        ));
        assert!(stale
            .reason()
            .starts_with("price unknown: the price list is 8 days old"));
        assert_eq!(
            unknown(estimate(
                Some(&catalog),
                NOW,
                "alibaba/wan-3.0",
                4,
                &p720,
                0
            )),
            UnknownPrice::NotListed
        );
        let mut bare = catalog.clone();
        bare.models[1].skus.clear();
        assert!(matches!(
            unknown(estimate(
                Some(&bare),
                NOW,
                "google/veo-3.1-lite",
                4,
                &p720,
                0
            )),
            UnknownPrice::NotListed | UnknownPrice::NoRate { .. }
        ));
        let mut rateless = catalog.clone();
        rateless
            .models
            .retain(|offer| offer.id == "google/veo-3.1-lite");
        rateless.models[0].skus.clear();
        assert_eq!(
            unknown(estimate(
                Some(&rateless),
                NOW,
                "google/veo-3.1-lite",
                4,
                &p720,
                0
            )),
            UnknownPrice::NoRate {
                resolution: "720p".into()
            }
        );
        assert_eq!(
            Price::Unknown(UnknownPrice::NoList).short(),
            "price unknown"
        );
    }

    #[test]
    fn the_shape_follows_the_output_and_refuses_what_the_model_cannot_make() {
        let catalog = catalog();
        let veo = catalog.offer("google/veo-3.1-lite").unwrap();
        let refused = shape(veo, (1080, 1080)).unwrap_err();
        assert!(
            refused.contains("16:9, 9:16") && refused.contains("1080x1080"),
            "{refused}"
        );
        let portrait = shape(veo, (1080, 1920)).unwrap();
        assert_eq!(
            (portrait.aspect_ratio.as_str(), portrait.size),
            ("9:16", (1080, 1920))
        );
        // A 4K project on a model that tops out at 1080p gets 1080p.
        assert_eq!(shape(veo, (3840, 2160)).unwrap().resolution, "1080p");
        // A tier between two listed ones rounds up.
        let mut two_k = veo.clone();
        two_k.resolutions = vec!["2K".into()];
        assert_eq!(shape(&two_k, (1920, 1080)).unwrap().size, (2560, 1440));
    }

    #[test]
    fn cents_round_up_and_never_down() {
        assert_eq!(ceil_cents(0.40), 0.40);
        assert_eq!(ceil_cents(0.401), 0.41);
        assert_eq!(ceil_cents(0.12 + 1e-12), 0.12);
        assert_eq!(ceil_cents(0.0), 0.0);
    }
}
