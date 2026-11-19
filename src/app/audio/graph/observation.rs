//! Opt-in source-frame witness at the existing pre-bus production boundary.
//! No sample tap, alternate mix, history or default per-sample atomic work.
use super::*;

pub(super) fn enabled(scene: &super::super::spatial::Scene) -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    scene.observes_sources()
        || *TRACE.get_or_init(|| std::env::var("DH2_DEBUG_AUDIO_RESOURCES").as_deref() == Ok("1"))
}

pub(super) struct Meter {
    blink_epoch:Option<Arc<std::sync::atomic::AtomicU64>>,
    blink_tick:u64,
    blink:Option<BlinkMeter>,
    sink:Option<super::super::observation::SourceSamples>,
    identity: Option<(u32, u32, Emitter)>,
    node: u32,
    frames: u64,
    decoded_peak: f32,
    dry_peak: f32,
    aux_peak: f32,
    speaker_gain_peak: f32,
    source_gain: f32,
    parent_gain: f32,
    route_bus: Option<u32>,
    route_next_block_gain: Option<f32>,
    master_next_block_gain: Option<f32>,
    reported: bool,
    lifetime: bool,
    route_target_peak:Option<f32>,
    master_target_peak:Option<f32>,
}
struct BlinkMeter {epoch:u64,frames:u64,decoded:f32,dry:f32,aux:f32,gain:f32,reported:bool}
impl Meter {
    pub(super) fn new(node: u32,sink:Option<super::super::observation::SourceSamples>,blink_epoch:Option<Arc<std::sync::atomic::AtomicU64>>) -> Self {
        Self {
            blink_epoch,blink_tick:u64::MAX,blink:None,
            lifetime:sink.is_some(),
            sink,
            identity: None,
            node,
            frames: 0,
            decoded_peak: 0.,
            dry_peak: 0.,
            aux_peak: 0.,
            speaker_gain_peak: 0.,
            source_gain: 0.,
            parent_gain: 0.,
            route_bus: None,
            route_next_block_gain: None,
            master_next_block_gain: None,
            reported: false,
            route_target_peak:None,
            master_target_peak:None,
        }
    }
    pub(super) fn begin_blink_interval(&mut self,tick:u64){
        if self.blink_tick==tick{return;}
        self.blink_tick=tick;
        //Only actual world-component voices establish ambient survival;
        //player/FX travel receipts cannot substitute for those emitters.
        if !self.identity.is_some_and(|(_,_,object)|matches!(object,Emitter::Component{..}|Emitter::ComponentJoint{..})){return;}
        let epoch=self.blink_epoch.as_ref().map_or(0,|epoch|epoch.load(Ordering::Relaxed));
        if epoch==0 {self.blink=None;}
        else if self.blink.as_ref().is_none_or(|value|value.epoch!=epoch){
            self.blink=Some(BlinkMeter{epoch,frames:0,decoded:0.,dry:0.,aux:0.,gain:0.,reported:false});
        }
    }
    pub(super) fn identify(&mut self, audio: &ScheduledAudio) {
        if self.identity.is_none() {
            self.identity = audio.event.as_ref().map(|event| {
                (
                    event.event().declaration,
                    event.id().value(),
                    event.event().object,
                )
            });
        }
    }
    pub(super) fn decoded(&mut self, samples: &[f32]) {
        if !self.reported||self.lifetime {
            for sample in samples {
                self.decoded_peak = self.decoded_peak.max(sample.abs());
                if let Some(blink)=&mut self.blink {blink.decoded=blink.decoded.max(sample.abs());}
            }
        }
    }
    pub(super) fn reported(&self) -> bool {
        //The offline observer follows the real source through Drop. Trace-only
        //mode keeps its existing onset-only cost and release behavior.
        self.reported&&!self.lifetime
    }
    pub(super) fn frame(
        &mut self,
        dry: &[f32],
        aux: f32,
        gains: &[f32],
        source_gain: f32,
        parent_gain: f32,
        route: Option<(u32, f32, f32)>,
    ) {
        if self.reported&&!self.lifetime {
            return;
        }
        self.frames += 1;
        if let Some(blink)=&mut self.blink {blink.frames+=1;blink.aux=blink.aux.max(aux);}
        for sample in dry {
            self.dry_peak = self.dry_peak.max(sample.abs());
            if let Some(blink)=&mut self.blink {blink.dry=blink.dry.max(sample.abs());}
        }
        self.aux_peak = self.aux_peak.max(aux);
        for gain in gains {
            self.speaker_gain_peak = self.speaker_gain_peak.max(gain.abs());
            if let Some(blink)=&mut self.blink {blink.gain=blink.gain.max(gain.abs());}
        }
        self.source_gain = source_gain;
        self.parent_gain = parent_gain;
        if let Some((bus, gain, master)) = route {
            self.route_bus = Some(bus);
            self.route_next_block_gain = Some(gain);
            self.master_next_block_gain = Some(master);
            self.route_target_peak=Some(self.route_target_peak.map_or(gain,|peak|peak.max(gain)));
            self.master_target_peak=Some(self.master_target_peak.map_or(master,|peak|peak.max(master)));
        }
        if !self.reported&&self.decoded_peak > 0. && (self.dry_peak > 0. || self.aux_peak > 0.) {
            self.report("first_nonzero");
        }
        if let (Some(blink),Some(sink),Some((event,playing,object)))=(&mut self.blink,&self.sink,self.identity){
            if !blink.reported&&blink.decoded>0.&&(blink.dry>0.||blink.aux>0.) {
                sink.lock().unwrap_or_else(|error|error.into_inner()).push(super::super::observation::SourceReceipt{
                    interval:blink.epoch,event,playing_id:playing,object,selected_source:self.node,frames:blink.frames,terminal:false,
                    route_bus:self.route_bus,route_target_peak:self.route_next_block_gain,master_target_peak:self.master_next_block_gain,
                    decoded_peak:blink.decoded,dry_peak:blink.dry,aux_peak:blink.aux,gain_peak:blink.gain,
                    route_next_block_gain:self.route_next_block_gain,master_next_block_gain:self.master_next_block_gain,
                    reason:"blink_window_first_nonzero_world_source",
                });
                blink.reported=true;
            }
        }
    }
    fn report(&mut self, reason: &'static str) {
        let Some((event, playing, object)) = self.identity else {
            return;
        };
        eprintln!("AUDIO_SOURCE_PROCESSED_SAMPLES event={event:#010x} playing_id={playing} object={object:?} selected_source_node={:#010x} frames={} decoded_peak={} post_voice_dry_peak={} post_voice_aux_peak={} speaker_ramp_gain_peak={} source_fade_gain={} parent_gain={} route_bus={:?} route_next_block_gain={:?} master_next_block_gain={:?} stage=post_voice_filters_gain_pre_bus_DSP final_bus_contribution_not_attributed=true reason={reason}",
            self.node,self.frames,self.decoded_peak,self.dry_peak,self.aux_peak,self.speaker_gain_peak,
            self.source_gain,self.parent_gain,self.route_bus,self.route_next_block_gain,self.master_next_block_gain);
        if let Some(sink)=&self.sink {sink.lock().unwrap_or_else(|error|error.into_inner()).push(super::super::observation::SourceReceipt{
            interval:0,event,playing_id:playing,object,selected_source:self.node,frames:self.frames,
            terminal:reason!="first_nonzero",route_bus:self.route_bus,
            route_target_peak:self.route_target_peak,master_target_peak:self.master_target_peak,
            decoded_peak:self.decoded_peak,
            dry_peak:self.dry_peak,aux_peak:self.aux_peak,gain_peak:self.speaker_gain_peak,
            route_next_block_gain:self.route_next_block_gain,master_next_block_gain:self.master_next_block_gain,reason,
        });}
        self.reported = true;
    }
}
impl Drop for Meter {
    fn drop(&mut self) {
        if self.lifetime||!self.reported {
            self.report(if self.frames==0 {
                "source_retired_before_post_gain_sample"
            }else if self.decoded_peak>0.&&(self.dry_peak>0.||self.aux_peak>0.){
                "source_retired_after_nonzero_post_gain"
            }else{
                "source_retired_without_nonzero_post_gain"
            });
        }
    }
}
