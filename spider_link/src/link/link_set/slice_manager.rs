use std::{cmp::min, collections::BTreeMap};

use crate::link::protocol::LinkProtocol;


pub struct SliceManager {
    epoch: u64,
    seq: u64,
    data: Vec<u8>,
    /// Maps the start of a slice to the index of its final element
    slices: BTreeMap<u64, u64>,
}

impl SliceManager {
    pub fn new(epoch: u64, seq: u64, size: u64) -> Self {
        Self {
            epoch,
            seq,
            data: vec![0; size as usize],
            slices: BTreeMap::new(),
        }
    }

    pub fn from_vec(epoch: u64, seq: u64, data: Vec<u8>) -> Self {
        let mut slices = BTreeMap::new();
        if data.len() > 0 {
            slices.insert(0, (data.len() - 1) as u64);
        }
        Self { epoch, seq, data, slices }
    }

    pub fn is_full(&self)->bool{
        if self.slices.len() == 1 {
            self.slices.first_key_value().unwrap() == (&0u64, &((self.data.len()-1) as u64))
        }else {
            false
        }
    }

    pub fn is_empty(&self)->bool{
        self.slices.is_empty()
    }

    pub fn data(&self) -> Option<&[u8]> {
        self.is_full().then_some(&self.data)
    }

    /// returns the last received index of the slice that begins at index 0.
    /// If such a slice exists
    pub fn last_head_index(&self) -> Option<u64> {
        self.slices.first_key_value().and_then(|(start, end)|{
            if *start != 0 {
                None
            }else{
                Some(end)
            }
        }).copied()
    }

    pub fn recv_protocol(&mut self, proto: &LinkProtocol) {
        match proto {
            LinkProtocol::Ack { epoch, seq, last_index , ..} => {
                if *epoch != self.epoch || *seq != self.seq {
                    return;
                }
                self.remove_slice(0, *last_index);
            }
            LinkProtocol::MsgSlice {
                epoch,
                seq,
                first_index,
                data,
                ..
            } => {
                if *epoch != self.epoch || *seq != self.seq {
                    return;
                }
                // cap the last_index at the end of the data.
                let last_index = proto.last_index().min(self.data.len() as u64 -1);
                if last_index < *first_index {
                    return; // cannot add such a slice
                }
                let length = last_index - first_index;

                self.add_slice(*first_index, last_index);
                let dest = &mut self.data[*first_index as usize..=last_index as usize];
                let src = &data[..=length as usize];
                dest.copy_from_slice(src);
            }
            _ => {}
        }
    }

    pub fn get_slices(&self, size: u32) -> Vec<LinkProtocol> {
        let mut ret = Vec::new();

        // get following slices
        for (next_start, next_end) in self.slices.iter() {
            let slices = make_slices(&self.data, self.epoch, self.seq, *next_start, size, *next_end);
            ret.extend(slices);
        }

        ret
    }

    fn remove_slice(&mut self, start: u64, end: u64) {
        // detect left hand overlaps, and splits
        if let Some((_, prev_end)) = self.slices.range_mut(..start).next_back() {
            // prev_start is before start. if prev_end is after end, the
            // slice to remove is entirely contained within this slice

            // truncate previous to before this slice
            let new_prev_end = start - 1;
            let old_prev_end = prev_end.to_owned();
            *prev_end = new_prev_end;

            if old_prev_end > end {
                // fully overlaps

                // since slice will be split, enter the part of the slice that
                // would be after the removed portion
                self.slices.insert(end + 1, old_prev_end);

                return; // since there was a full overlap, there is no other overlap.
            }
        }

        // detect interior overlaps and right hand overlaps
        let mut cursor = start;
        while let Some((next_start, next_end)) = self.slices.range_mut(cursor..).next() {
            cursor = *next_end;

            if *next_end <= end {
                // if slice is fully contained, remove it
                let new_start = next_start.to_owned();
                self.slices.remove(&new_start);
            } else {
                // otherwise, update the start point
                let new_start = next_start.to_owned();
                let new_end = next_end.to_owned();
                self.slices.remove(&new_start);
                self.slices.insert(end + 1, new_end);
            }

            if cursor > end {
                break;
            }
        }
    }

    fn add_slice(&mut self, start: u64, mut end: u64) {
        // detect interior and right hand overlaps
        let mut cursor = start;
        while let Some((next_start, next_end)) = self.slices.range_mut(cursor..).next() {
            cursor = *next_end;

            if *next_end <= end {
                // if slice is fully contained, remove it
                let new_start = next_start.to_owned();
                self.slices.remove(&new_start);
            } else {
                // otherwise, update our end point
                let new_start = next_start.to_owned();
                end = next_end.to_owned();

                self.slices.remove(&new_start);
                break;
            }

            if cursor > end {
                break;
            }
        }

        // detect left hand overlaps
        if let Some((_, prev_end)) = self.slices.range_mut(..start).next_back() {
            // If previous end is after new end, no work is needed
            if *prev_end >= end {
                return;
            }

            // If previous end is within the new slice or adjacent to it, set it
            // to the end
            if *prev_end >= start.saturating_sub(1) {
                *prev_end = end;
                return;
            }
        }

        // Insert new slice
        self.slices.insert(start, end);
    }
}


fn make_slices(data: &Vec<u8>, epoch: u64, seq: u64, start: u64, size: u32, end: u64) -> Vec<LinkProtocol> {
    if size == 0 {
        return Vec::new();
    }
    let mut ret = Vec::new();
    let mut cursor = start;
    loop {
        let slice_end = min(min(cursor + size as u64 - 1, end) as usize, data.len() - 1);
        if slice_end < cursor as usize {
            break;
        }
        let slice_data = data.get(cursor as usize..=slice_end).unwrap();

        ret.push(LinkProtocol::MsgSlice {
            epoch,
            seq,
            first_index: cursor,
            seq_len: data.len() as u64,
            data: slice_data.to_owned(),
        });

        cursor += size as u64;
        if cursor > end {
            break;
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use rand::{seq::SliceRandom, thread_rng};

    use super::*;

    #[test]
    fn add_slice_empty() {
        let data = vec![0; 100];
        let slices = BTreeMap::new();
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.add_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(30, 70)]
        )
    }

    #[test]
    fn add_slice_split() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(0, 40 as u64);
        slices.insert(60, 99 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.add_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(0, 99)]
        )
    }

    #[test]
    fn add_slice_left_hand_overlap() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(0, 50 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.add_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(0, 70)]
        )
    }

    #[test]
    fn add_slice_right_hand_overlap() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(50, 99 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.add_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(30, 99)]
        )
    }

    #[test]
    fn add_slice_full_overlap() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(30, 70 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.add_slice(40, 60);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(30, 70)]
        )
    }

    #[test]
    fn add_slice_exact_overlap() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(30, 70 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.add_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(30, 70)]
        )
    }

    #[test]
    fn add_slice_off_by_one_left() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(29, 29 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.add_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(29, 70)]
        )
    }

    #[test]
    fn add_slice_off_by_one_right() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(71, 71 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.add_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(30, 71)]
        )
    }

    #[test]
    fn add_slice_multiple_regions() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(5, 15 as u64);
        slices.insert(25, 35 as u64);
        slices.insert(45, 55 as u64);
        slices.insert(65, 75 as u64);
        slices.insert(85, 95 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.add_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(5, 15), (25, 75), (85, 95)]
        )
    }

    #[test]
    fn remove_slice_split() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(0, data.len() as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.remove_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(0, 29), (71, 100)]
        )
    }

    #[test]
    fn remove_slice_left_hand_overlap() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(0, 50 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.remove_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(0, 29)]
        )
    }

    #[test]
    fn remove_slice_right_hand_overlap() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(50, 100 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.remove_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(71, 100)]
        )
    }

    #[test]
    fn remove_slice_full_overlap() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(40, 60 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.remove_slice(30, 70);

        assert_eq!(mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(), vec![])
    }

    #[test]
    fn remove_slice_exact_overlap() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(30, 70 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.remove_slice(30, 70);

        assert_eq!(mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(), vec![])
    }

    #[test]
    fn remove_slice_off_by_one_left() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(29, 70 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.remove_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(29, 29)]
        )
    }

    #[test]
    fn remove_slice_off_by_one_right() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(30, 71 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.remove_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(71, 71)]
        )
    }

    #[test]
    fn remove_slice_multiple_regions() {
        let data = vec![0; 100];
        let mut slices = BTreeMap::new();
        slices.insert(5, 15 as u64);
        slices.insert(25, 35 as u64);
        slices.insert(45, 55 as u64);
        slices.insert(65, 75 as u64);
        slices.insert(85, 95 as u64);
        let mut mgr = SliceManager {epoch: 2, seq: 28, data, slices };

        mgr.remove_slice(30, 70);

        assert_eq!(
            mgr.slices.into_iter().collect::<Vec<(u64, u64)>>(),
            vec![(5, 15), (25, 29), (71, 75), (85, 95)]
        )
    }

    #[test]
    fn make_slices_simple() {
        let mgr = SliceManager::from_vec(4, 50, b"test data".into());

        // println!("data: {:?}", mgr.data);
        let start = 5;
        let end = (mgr.data.len() - 1) as u64;
        let slices = make_slices(&mgr.data, 4, 39, start, 3, end);
        // println!("start: {start} end: {end}");
        // println!("out: {:?}",  slices);

        assert_eq!(
            slices.iter().map(|proto| { proto.data().to_owned() }).collect::<Vec<Vec<u8>>>(),
            vec![vec![100, 97, 116], vec![97]]
        )
    }

    #[test]
    fn make_slices_len() {
        let mgr = SliceManager::from_vec(4, 50, b"test data".into());

        // println!("data: {:?}", mgr.data);
        let start = 5;
        let end = (mgr.data.len() - 1) as u64;
        let slices = make_slices(&mgr.data, 4, 39, start, 3, end);
        // println!("start: {start} end: {end}");
        // println!("out: {:?}",  slices);

        assert_eq!(
            slices.len() as u64,
            end-start-1
        )
    }

    #[test]
    fn make_slices_seq() {
        let epoch = 4;
        let mgr = SliceManager::from_vec(epoch, 50, b"test data".into());

        // println!("data: {:?}", mgr.data);
        let seq = 35;
        let start = 5;
        let end = (mgr.data.len() - 1) as u64;
        let slices = make_slices(&mgr.data, epoch, seq, start, 3, end);
        // println!("start: {start} end: {end}");
        // println!("out: {:?}",  slices);

        for proto in slices {
            assert_eq!(
                proto.seq(),
                seq
            )
        }
        
    }

    #[test]
    fn make_slices_proto_len() {
        let epoch = 4;
        let seq = 35;
        let test_data: Vec<u8> = b"test data".into();
        let data_len = test_data.len();
        let mgr = SliceManager::from_vec(epoch, seq, test_data);

        // println!("data: {:?}", mgr.data);
        let start = 5;
        let end = (mgr.data.len() - 1) as u64;
        let slices = make_slices(&mgr.data, epoch, seq, start, 3, end);
        // println!("start: {start} end: {end}");
        // println!("out: {:?}",  slices);

        for proto in slices {
            assert_eq!(
                data_len as u64,
                proto.seq_len()
            )
        }
        
    }

    #[test]
    fn make_slices_end_before_start() {
        let epoch = 4;
        let seq = 65;
        let mgr = SliceManager::from_vec(epoch, seq, b"test data".into());

        // println!("data: {:?}", mgr.data);
        let start = 5;
        let end = 4;
        let slices = make_slices(&mgr.data, epoch, seq, start, 3, end);
        // println!("start: {start} end: {end}");
        // println!("out: {:?}",  slices);

        assert_eq!(
            slices.iter().map(|proto| { proto.data().to_owned() }).collect::<Vec<Vec<u8>>>(),
            Vec::<Vec<u8>>::new()
        )
    }

    #[test]
    fn make_slices_zero_sized() {
        let epoch = 6;
        let seq = 342;
        let mgr = SliceManager::from_vec(epoch, seq, b"test data".into());

        // println!("data: {:?}", mgr.data);
        let start = 5;
        let end = 7;
        let slices = make_slices(&mgr.data, epoch, seq, start, 0, end);
        // println!("start: {start} end: {end}");
        // println!("out: {:?}",  slices);

        assert_eq!(
            slices.iter().map(|proto| { proto.data().to_owned() }).collect::<Vec<Vec<u8>>>(),
            Vec::<Vec<u8>>::new()
        )
    }

    #[test]
    fn is_full_new() {
        let mgr = SliceManager::from_vec(4, 50, b"test data".into());

        assert!(mgr.is_full());
        assert!(!mgr.is_empty());
    }

    #[test]
    fn is_full_one_slice() {
        let mut mgr = SliceManager::new(4, 50, 20);
        mgr.add_slice(5, 10);

        assert!(!mgr.is_full());
        assert!(!mgr.is_empty());
    }

    #[test]
    fn is_full_two_slice() {
        let mut mgr = SliceManager::new(4, 50, 20);
        mgr.add_slice(5, 10);
        mgr.add_slice(15, 18);

        assert!(!mgr.is_full());
        assert!(!mgr.is_empty());
    }

    #[test]
    fn is_empty_new() {
        let mgr = SliceManager::new(4, 50, 10);

        assert!(mgr.is_empty());
        assert!(!mgr.is_full());
    }

    #[test]
    fn transfer_slices() {
        let epoch = 4;
        let seq = 50;
        let data = b"Here is some test data to be sent!";
        let sending_mgr = SliceManager::from_vec(epoch, seq, data.to_vec());
        let mut receiving_mgr = SliceManager::new(epoch, seq, data.len() as u64);

        let mut slices = sending_mgr.get_slices(3);
        slices.shuffle(&mut thread_rng());
        for slice in slices {
            receiving_mgr.recv_protocol(&slice);
        }

        let recvd_data = receiving_mgr.data().expect("all data should have been sent");
        assert_eq!(data, recvd_data);
    }

    #[test]
    fn transfer_slice_past_the_end() {
        let epoch = 506;
        let seq = 1;
        let data = b"Here is some test data to be sent!";
        let sending_mgr = SliceManager::from_vec(epoch, seq, data.to_vec());
        let mut receiving_mgr = SliceManager::new(epoch, seq, data.len() as u64);

        let slices = sending_mgr.get_slices(3);
        for slice in slices {
            receiving_mgr.recv_protocol(&slice);
        }

        receiving_mgr.recv_protocol(&LinkProtocol::MsgSlice { epoch: 4, seq: 1, seq_len: 5, first_index: 34, data: b"test!".to_vec() });

        let recvd_data = receiving_mgr.data().expect("all data should have been sent");
        assert_eq!(data, recvd_data);
    }

}
