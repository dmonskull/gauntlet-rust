//! Decodes every frame of a movie and compares it with FFmpeg's decode
//! (`ffmpeg -i <movie> -f rawvideo -pix_fmt yuv444p <raw>`): planes Y, U, V
//! per frame.
//! Usage: moviecheck <movie.avi> [<ffmpeg yuv444p raw>]
use gdl_formats::movie::{Movie, MvdvDecoder};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: moviecheck <movie.avi> [raw]");
    let movie = Movie::parse(std::fs::read(&path).expect("read movie")).expect("parse movie");
    println!(
        "{}x{} at {} fps, {} frames, sound {} bytes at {} Hz × {} ch, {} bits",
        movie.width,
        movie.height,
        movie.rate,
        movie.frame_count(),
        movie.sound.len(),
        movie.sample_rate,
        movie.channels,
        movie.bits
    );
    let reference = args.next().map(|p| std::fs::read(p).expect("read raw"));
    let mut d = MvdvDecoder::new(movie.width, movie.height);
    let plane = movie.width * movie.height;
    let (mut keys, mut bad) = (0, 0);
    for i in 0..movie.frame_count() {
        match d.decode(movie.frame(i).unwrap()) {
            Ok(key) => keys += usize::from(key),
            Err(e) => {
                println!("frame {i}: {e}");
                bad += 1;
                continue;
            }
        }
        if let Some(raw) = &reference {
            let at = i * plane * 3;
            let Some(f) = raw.get(at..at + plane * 3) else { continue };
            let differ = [(&d.y, 0), (&d.u, 1), (&d.v, 2)]
                .iter()
                .map(|(p, k)| p.iter().zip(&f[k * plane..(k + 1) * plane]).filter(|(a, b)| a != b).count())
                .sum::<usize>();
            if differ > 0 {
                println!("frame {i}: {differ} samples differ from FFmpeg's");
                bad += 1;
            }
        }
    }
    println!("{} frames, {keys} key frames, {bad} bad", movie.frame_count());
}
