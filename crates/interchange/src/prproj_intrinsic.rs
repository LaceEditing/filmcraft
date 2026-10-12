//! Premiere's intrinsic clip components (Motion, Opacity and clip Volume), written the way Premiere
//! writes them once a clip's value leaves the default. Until then Premiere saves no component at
//! all, only `DefaultMotion` / `DefaultOpacity` / `DefaultVol` on the clip's chain. A new project
//! and a clip whose Motion, Opacity or Volume was first changed in FilmCraft both need them.

use crate::xml::escape;

/// `StartKeyframe` time of a static value.
pub(crate) const NO_TIME: &str = "-91445760000000000";
pub(crate) const STEREO: &str = r#"[{"channellabel":100},{"channellabel":101}]"#;
pub(crate) const MONO: &str = r#"[{"channellabel":0}]"#;
/// Clip volume at 0 dB as Premiere stores it (linear, +15 dB of headroom above 1.0), to the
/// precision FilmCraft reads it with (`prproj_fx::UNITY_LEVEL`).
pub(crate) const UNITY_LEVEL: &str = "0.1778279410038923";

/// Class ids (and object versions) of the component objects.
pub(crate) mod class {
    pub const AUDIO_PARAM: (&str, u32) = ("a714635e-a628-4b27-9d59-77eba47dbc1a", 10);
    pub const AUDIO_BOOL_PARAM: (&str, u32) = ("32657501-3aa4-445f-a49b-d09ecb9fa1ae", 10);
    pub const AUDIO_FILTER: (&str, u32) = ("d77a90a0-6c9e-44bf-9b20-de8c21168fe1", 4);
    pub const VIDEO_FILTER: (&str, u32) = ("d10da199-beea-4dd1-b941-ed3a78766d50", 9);
    pub const POINT_PARAM: (&str, u32) = ("ca81d347-309b-44d2-acc7-1c572efb973c", 4);
    pub const FLOAT_PARAM: (&str, u32) = ("fe47129e-6c94-4fc0-95d5-c056a517aaf3", 10);
    pub const BOOL_PARAM: (&str, u32) = ("cc12343e-f113-4d3b-ae05-b287db77d461", 10);
    pub const SLIDER_PARAM: (&str, u32) = ("a4ff2d6e-7ac2-44f8-9d52-17d9ca50e542", 10);
    pub const POPUP_PARAM: (&str, u32) = ("6e02e8bb-2569-46b2-8ab1-4ab11c43e9c8", 10);
}

/// Somewhere new objects go: `obj` adds one (`<tag ObjectID=… ClassID=… Version=…>body</tag>`)
/// and returns its id.
pub(crate) trait Objects {
    fn obj(&mut self, tag: &str, class: (&str, u32), body: &str) -> u64;
}

fn el(name: &str, value: &str) -> String {
    format!("<{name}>{}</{name}>", escape(value))
}

/// A list element (`<Params Version="1"><Param Index="0" ObjectRef="…"/>…</Params>`).
pub(crate) fn list<T: std::fmt::Display>(tag: &str, item: &str, refs: &[T]) -> String {
    let items: String = refs.iter().enumerate().map(|(i, r)| format!("<{item} Index=\"{i}\" ObjectRef=\"{r}\"/>")).collect();
    format!("<{tag} Version=\"1\">{items}</{tag}>")
}

fn float(o: &mut impl Objects, name: &str, pid: u32, v: &str, bounds: &str, extra: &str) -> u64 {
    o.obj(
        "VideoComponentParam",
        class::FLOAT_PARAM,
        &format!("{}{extra}<ParameterID>{pid}</ParameterID><StartKeyframe>{NO_TIME},{v},0,0,0,0,0,0</StartKeyframe>{bounds}", el("Name", name)),
    )
}

fn point(o: &mut impl Objects, name: &str, pid: u32) -> u64 {
    o.obj(
        "PointComponentParam",
        class::POINT_PARAM,
        &format!("{}<ParameterID>{pid}</ParameterID><StartKeyframe>{NO_TIME},0.5:0.5,0,0,0,0,0,0,5,4,0,0,0,0</StartKeyframe>", el("Name", name)),
    )
}

/// Motion at its defaults (centred, 100 %), with component id `id`.
pub(crate) fn motion(o: &mut impl Objects, id: u32) -> u64 {
    let scale_bounds = "<LowerBound>0</LowerBound><UpperBound>10000</UpperBound>";
    let pct = "<LowerBound>0</LowerBound><UpperBound>100</UpperBound>";
    let params = vec![
        point(o, "Position", 1),
        float(o, "Scale", 2, "100.", scale_bounds, "<UpperUIBound>200</UpperUIBound>"),
        float(o, "Scale Width", 3, "100.", scale_bounds, "<UpperUIBound>200</UpperUIBound>"),
        // Uniform Scale: Premiere saves it without a name
        o.obj("VideoComponentParam", class::BOOL_PARAM, &format!("<Name> </Name><ParameterID>4</ParameterID><StartKeyframe>{NO_TIME},true,0,0,0,0,0,0</StartKeyframe>")),
        float(o, "Rotation", 5, "0.", "<LowerBound>-32768</LowerBound><UpperBound>32767</UpperBound>", "<ParameterControlType>3</ParameterControlType>"),
        point(o, "Anchor Point", 6),
        o.obj(
            "VideoComponentParam",
            class::SLIDER_PARAM,
            &format!("<Name>Anti-flicker Filter</Name><ParameterID>7</ParameterID><StartKeyframe>{NO_TIME},0.,0,0,0,0,0,0</StartKeyframe><LowerBound>0</LowerBound><UpperBound>1</UpperBound>"),
        ),
        float(o, "Crop Left", 8, "0.", pct, ""),
        float(o, "Crop Top", 9, "0.", pct, ""),
        float(o, "Crop Right", 10, "0.", pct, ""),
        float(o, "Crop Bottom", 11, "0.", pct, ""),
    ];
    o.obj(
        "VideoFilterComponent",
        class::VIDEO_FILTER,
        &format!(
            "<Component Version=\"7\">{}<ID>{id}</ID><DisplayName>Motion</DisplayName><Intrinsic>true</Intrinsic></Component><PremiereFilterPrivateData Encoding=\"base64\" BinaryHash=\"3805854c-243a-c4be-bd40-a8a40000000e\">AQA=</PremiereFilterPrivateData><MatchName>AE.ADBE Motion</MatchName><VideoFilterType>2</VideoFilterType>",
            list("Params", "Param", &params)
        ),
    )
}

/// Opacity at its defaults (100 %, Normal), with component id `id`.
pub(crate) fn opacity(o: &mut impl Objects, id: u32) -> u64 {
    let params = vec![
        o.obj(
            "VideoComponentParam",
            class::FLOAT_PARAM,
            &format!("<Name>Opacity</Name><ParameterID>1</ParameterID><StartKeyframe>{NO_TIME},100.,0,0,0,0,0,0</StartKeyframe><LowerBound>0</LowerBound><UpperBound>100</UpperBound>"),
        ),
        o.obj(
            "VideoComponentParam",
            class::POPUP_PARAM,
            &format!("<Name>Blend Mode</Name><ParameterControlType>10</ParameterControlType><ParameterID>2</ParameterID><DiscontinuousInterpolate>true</DiscontinuousInterpolate><StartKeyframe>{NO_TIME},18,0,0,0,0,0,0</StartKeyframe><LowerBound>0</LowerBound><UpperBound>27</UpperBound>"),
        ),
        o.obj(
            "VideoComponentParam",
            class::POPUP_PARAM,
            &format!("<Name>Blend Mode</Name><ParameterID>3</ParameterID><DiscontinuousInterpolate>true</DiscontinuousInterpolate><StartKeyframe>{NO_TIME},0,0,0,0,0,0,0</StartKeyframe><LowerBound>0</LowerBound><UpperBound>31</UpperBound>"),
        ),
    ];
    o.obj(
        "VideoFilterComponent",
        class::VIDEO_FILTER,
        &format!(
            "<Component Version=\"7\">{}<ID>{id}</ID><DisplayName>Opacity</DisplayName><Intrinsic>true</Intrinsic></Component><MatchName>AE.ADBE Opacity</MatchName><VideoFilterType>2</VideoFilterType>",
            list("Params", "Param", &params)
        ),
    )
}

/// A clip volume level parameter at 0 dB (`name`: None for Channel Volume's unnamed slots).
pub(crate) fn audio_level(o: &mut impl Objects, name: Option<&str>) -> u64 {
    let name = name.map(|n| el("Name", n)).unwrap_or_default();
    o.obj(
        "AudioComponentParam",
        class::AUDIO_PARAM,
        &format!("<StartKeyframe>{NO_TIME},{UNITY_LEVEL},0,0,0,0,0,0</StartKeyframe><CurrentValue>{UNITY_LEVEL}</CurrentValue><IsTimeVarying>false</IsTimeVarying><UnitsString>dB</UnitsString>{name}"),
    )
}

/// An off switch (Mute, Bypass).
pub(crate) fn audio_toggle(o: &mut impl Objects, name: &str) -> u64 {
    o.obj("AudioComponentParam", class::AUDIO_BOOL_PARAM, &format!("<IsTimeVarying>false</IsTimeVarying><RangeLocked>false</RangeLocked>{}", el("Name", name)))
}

/// An intrinsic audio filter (`Internal Volume`, `Internal Channel Volume`) of a mono or stereo
/// clip; `sample` is ticks per audio sample.
pub(crate) fn audio_filter(o: &mut impl Objects, params: &[u64], id: u32, name: &str, mono: bool, sample: i64) -> u64 {
    let (layout, kind, config, suffix) = if mono {
        (MONO, 0, r#"{"in":[{"layout":[0],"name":"Mono In","type":0}],"out":[{"layout":[0],"name":"Mono Out","type":0}]}"#, "Mono")
    } else {
        (STEREO, 1, r#"{"in":[{"layout":[100,101],"name":"Stereo In","type":0}],"out":[{"layout":[100,101],"name":"Stereo Out","type":0}]}"#, "Stereo")
    };
    o.obj(
        "AudioFilterComponent",
        class::AUDIO_FILTER,
        &format!(
            "<AudioComponent Version=\"3\"><Component Version=\"7\">{}<Intrinsic>true</Intrinsic><ID>{id}</ID></Component>{}<ChannelType>{kind}</ChannelType><FrameRate>{sample}</FrameRate><AudioComponentType>0</AudioComponentType></AudioComponent><FilterPreset>0</FilterPreset>{}<FilterIndex>-1</FilterIndex>{}",
            list("Params", "Param", params),
            el("AudioChannelLayout", layout),
            el("FilterMatchName", &format!("{name} {suffix}")),
            el("ChannelConfigData", config)
        ),
    )
}

/// Clip volume at 0 dB (`Internal Volume Mono|Stereo`), with component id `id`.
pub(crate) fn volume(o: &mut impl Objects, id: u32, mono: bool, sample: i64) -> u64 {
    let mute = audio_toggle(o, "Mute");
    let level = audio_level(o, Some("Level"));
    audio_filter(o, &[mute, level], id, "Internal Volume", mono, sample)
}
